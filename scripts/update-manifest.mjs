// Writes the update manifest of one package (latest-linux-x86_64.json, latest-macos.json) in the
// same format as the Windows latest.json made by scripts/build-installer.ps1.
// Usage: node scripts/update-manifest.mjs <tag> <asset name> <signature file> <output>
import { readFileSync, writeFileSync } from "node:fs";

const [tag, asset, sigFile, output] = process.argv.slice(2);
if (!tag || !asset || !sigFile || !output) {
  console.error("Uso: node scripts/update-manifest.mjs <tag> <asset> <arquivo .sig> <saída>");
  process.exit(2);
}
const version = tag.replace(/^v/, "");
const manifest = {
  version,
  notes: `https://github.com/aekasota/BOLADAO/releases/tag/${tag}`,
  pub_date: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
  url: `https://github.com/aekasota/BOLADAO/releases/download/${tag}/${asset}`,
  signature: readFileSync(sigFile, "utf8").trim(),
};
// UTF-8 without BOM: the app's JSON reader does not take a BOM.
writeFileSync(output, JSON.stringify(manifest, null, 2));
console.log(`${output}: ${version}`);
