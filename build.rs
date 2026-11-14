fn main() {
    // Sert à choisir le bon fichier lors des mises à jour automatiques.
    println!("cargo:rustc-env=ECHOES_TARGET={}", std::env::var("TARGET").unwrap());
}
