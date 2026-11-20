#!/usr/bin/env node
// Rend les PNG et l'icône macOS (.icns) à partir de brand/echoes-icone.svg.
//   python3 brand/generer.py && node brand/rendre.js
// Besoin : Node et Playwright (Chromium). Les fichiers produits sont versionnés : la publication
// n'a pas à refaire ce rendu, elle copie packaging/macos/Echoes.icns dans Echoes.app.
const fs = require("fs");
const path = require("path");
const { chromium } = require("playwright");

const ici = __dirname;
const racine = path.join(ici, "..");

// Types ICNS modernes (PNG). Chaque entrée : code, taille en pixels.
const ICNS = [
  ["icp4", 16], ["icp5", 32], ["icp6", 64], ["ic07", 128], ["ic08", 256], ["ic09", 512], ["ic10", 1024],
  ["ic11", 32], ["ic12", 64], ["ic13", 256], ["ic14", 512],
];

async function rendu(page, fichier, taille) {
  const svg = fs.readFileSync(path.join(ici, fichier), "utf8")
    .replace(/width="\d+" height="\d+"/, `width="${taille}" height="${taille}"`);
  await page.setViewportSize({ width: taille, height: taille });
  await page.setContent(`<style>html,body{margin:0;background:transparent}svg{display:block}</style>${svg}`);
  return page.screenshot({ omitBackground: true, clip: { x: 0, y: 0, width: taille, height: taille } });
}

function icns(entrees) {
  const blocs = entrees.map(([code, png]) => {
    const tete = Buffer.alloc(8);
    tete.write(code, 0, "ascii");
    tete.writeUInt32BE(png.length + 8, 4);
    return Buffer.concat([tete, png]);
  });
  const corps = Buffer.concat(blocs);
  const tete = Buffer.alloc(8);
  tete.write("icns", 0, "ascii");
  tete.writeUInt32BE(corps.length + 8, 4);
  return Buffer.concat([tete, corps]);
}

(async () => {
  const navigateur = await chromium.launch();
  const page = await navigateur.newPage();
  const cache = {};
  const png = async (fichier, t) => (cache[fichier + t] ??= await rendu(page, fichier, t));

  // Sous 64 px, le dessin simplifié (favicon) reste lisible ; au-delà, l'icône complète.
  const entrees = [];
  for (const [code, t] of ICNS) entrees.push([code, await png(t < 64 ? "favicon.svg" : "echoes-icone.svg", t)]);
  fs.writeFileSync(path.join(racine, "packaging/macos/Echoes.icns"), icns(entrees));

  const web = path.join(racine, "docs/assets");
  fs.mkdirSync(web, { recursive: true });
  fs.writeFileSync(path.join(web, "echoes-icone-512.png"), await png("echoes-icone.svg", 512));
  fs.writeFileSync(path.join(web, "apple-touch-icon.png"), await png("echoes-icone.svg", 180));
  for (const f of ["echoes-icone.svg", "favicon.svg", "echoes-glyphe.svg", "echoes-glyphe-mono.svg"]) {
    fs.copyFileSync(path.join(ici, f), path.join(web, f));
  }
  await navigateur.close();
  console.log("Echoes.icns et images web écrits.");
})();
