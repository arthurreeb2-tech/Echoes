//! Icône Echoes dans la barre des menus (macOS) ou la zone de notification (Windows).

pub mod model;

use anyhow::Result;

/// Mémoire du fil de rafraîchissement : adresses déjà trouvées par Bonjour et date de la dernière recherche.
/// L'icône n'écrit jamais le cache `peers` de `echoes run` et ne lance Bonjour que rarement (`model::should_browse`).
#[derive(Default)]
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
struct Scan {
    seen: Vec<std::net::SocketAddr>,
    last_browse: Option<std::time::Instant>,
}

/// Mesure l'état du réseau, sans interface : appelé toutes les 15 s par le fil de rafraîchissement.
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
fn snapshot(scan: &mut Scan) -> model::Summary {
    use crate::{client, config, dashboard::Machine, discover, history, status};
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let me = status::local().ok();
    let key = config::load_key().ok().flatten();
    let known: Vec<Machine> = config::data_dir()
        .ok()
        .and_then(|d| std::fs::read(d.join("machines.json")).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let (peers, up) = match &key {
        Some(k) => {
            let my_id = me.as_ref().map(|m| m.id.clone()).unwrap_or_default();
            let mut addrs = client::known_addrs();
            addrs.extend(scan.seen.iter().copied());
            addrs.sort();
            addrs.dedup();
            let mut found = client::probe_addrs(&addrs, k, &my_id);
            if model::should_browse(!found.is_empty(), scan.last_browse.map(|t| t.elapsed())) {
                scan.last_browse = Some(std::time::Instant::now());
                for a in discover::browse(std::time::Duration::from_millis(800)) {
                    if !scan.seen.contains(&a) {
                        scan.seen.push(a);
                    }
                }
                addrs.extend(scan.seen.iter().copied());
                addrs.sort();
                addrs.dedup();
                found = client::probe_addrs(&addrs, k, &my_id);
            }
            (found.into_iter().map(|(_, s)| s).collect(), client::service_up(k))
        }
        None => (vec![], false),
    };
    model::summarize(me.as_ref(), &known, &peers, &history::read(2000), up, key.is_some(), now)
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn run() -> Result<i32> {
    println!("L'icône de la barre des menus n'est pas disponible sous Linux.");
    Ok(0)
}

/// Une session graphique est-elle ouverte ? Sans elle (connexion SSH), aucune icône n'est possible.
#[cfg(target_os = "macos")]
fn has_display() -> bool {
    use std::ffi::c_void;
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGSessionCopyCurrentDictionary() -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *const c_void);
    }
    // SAFETY : fonction sans argument ; le dictionnaire renvoyé (s'il existe) nous appartient et est libéré aussitôt.
    unsafe {
        let dict = CGSessionCopyCurrentDictionary();
        if dict.is_null() {
            return false;
        }
        CFRelease(dict);
    }
    true
}

#[cfg(windows)]
fn has_display() -> bool {
    true
}

#[cfg(any(target_os = "macos", windows))]
pub fn run() -> Result<i32> {
    use tao::event::{Event, StartCause};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    if !has_display() {
        eprintln!("echoes: pas de session graphique, l'icône n'est pas affichée");
        return Ok(0);
    }

    enum UserEvent {
        Refresh(Box<model::Summary>),
        Menu(MenuEvent),
    }

    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }

    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |e| {
        let _ = proxy.send_event(UserEvent::Menu(e));
    }));
    let proxy = event_loop.create_proxy();
    std::thread::spawn(move || {
        let mut scan = Scan::default();
        loop {
            if proxy.send_event(UserEvent::Refresh(Box::new(snapshot(&mut scan)))).is_err() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_secs(15));
        }
    });

    let icon_for = |light: model::Light| {
        let colour = match light {
            model::Light::Green => crate::icon::GREEN,
            model::Light::Orange => crate::icon::ORANGE,
            model::Light::Grey => crate::icon::GREY,
        };
        // 18 points en résolution double.
        Icon::from_rgba(crate::icon::render(36, Some(colour), false), 36, 36).ok()
    };
    let open = MenuItem::new("Ouvrir le tableau de bord", true, None);
    let quit = MenuItem::new("Quitter", true, None);
    let (open_id, quit_id) = (open.id().clone(), quit.id().clone());
    let build_menu = move |s: Option<&model::Summary>| {
        let menu = Menu::new();
        match s {
            Some(s) => {
                for row in &s.machines {
                    let _ = menu.append(&MenuItem::new(if row.online { format!("● {}", row.label) } else { format!("○ {}", row.label) }, false, None));
                }
                let _ = menu.append(&MenuItem::new(&s.activity, false, None));
            }
            None => {
                let _ = menu.append(&MenuItem::new("Mesure en cours…", false, None));
            }
        }
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&open);
        let _ = menu.append(&quit);
        menu
    };

    let mut tray: Option<TrayIcon> = None;
    let mut shown: Option<model::Summary> = None;
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            // Sur macOS, l'icône doit être créée une fois la boucle démarrée.
            Event::NewEvents(StartCause::Init) => {
                let built = TrayIconBuilder::new()
                    .with_menu(Box::new(build_menu(None)))
                    .with_tooltip("Echoes")
                    .with_icon(icon_for(model::Light::Grey).expect("icône valide"))
                    .build();
                match built {
                    Ok(t) => tray = Some(t),
                    Err(e) => {
                        eprintln!("echoes: icône impossible à afficher : {e}");
                        *control_flow = ControlFlow::ExitWithCode(0);
                    }
                }
            }
            Event::UserEvent(UserEvent::Refresh(s)) => {
                if !model::menu_changed(shown.as_ref(), &s) {
                    return;
                }
                // Journal (~/Library/Logs/echoes-tray.log) : une ligne à chaque changement d'état.
                if shown.as_ref().map(|p| &p.tooltip) != Some(&s.tooltip) {
                    eprintln!("echoes: icône {:?} : {}", s.light, s.tooltip);
                }
                if let Some(t) = &tray {
                    if shown.as_ref().map(|p| p.light) != Some(s.light) {
                        let _ = t.set_icon(icon_for(s.light));
                    }
                    let _ = t.set_tooltip(Some(&s.tooltip));
                    t.set_menu(Some(Box::new(build_menu(Some(&s)))));
                }
                shown = Some(*s);
            }
            Event::UserEvent(UserEvent::Menu(e)) if e.id == open_id => open_dashboard(),
            Event::UserEvent(UserEvent::Menu(e)) if e.id == quit_id => {
                tray.take();
                *control_flow = ControlFlow::ExitWithCode(0);
            }
            _ => {}
        }
    })
}

/// Tableau de bord dans un processus séparé : `dashboard::open` peut servir la page lui-même
/// puis terminer le processus, ce qui fermerait l'icône.
#[cfg(any(target_os = "macos", windows))]
fn open_dashboard() {
    let bin = crate::install::bin_path().ok().filter(|p| p.exists()).or_else(|| std::env::current_exe().ok());
    if let Some(bin) = bin {
        let mut cmd = std::process::Command::new(bin);
        cmd.arg("dashboard").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let _ = cmd.spawn();
    }
}
