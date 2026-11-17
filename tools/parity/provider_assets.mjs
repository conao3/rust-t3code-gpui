import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";

const [root, destination] = process.argv.slice(2);
if (!root || !destination) throw new Error("Expected original source root and destination");
fs.mkdirSync(destination, { recursive: true });
const sources = {};
const read = relative => {
  const source = fs.readFileSync(path.join(root, relative), "utf8");
  sources[relative] = createHash("sha256").update(source).digest("hex");
  return source;
};
const escape = value => String(value).replaceAll("&", "&amp;").replaceAll('"', "&quot;").replaceAll("<", "&lt;");
const generated = [];
for (const name of fs.readdirSync(path.join(root, "packages")).filter(name => name.startsWith("provider-"))) {
  const relative = `packages/${name}/src/client.ts`;
  if (!fs.existsSync(path.join(root, relative))) continue;
  const source = read(relative);
  const literal = source.match(/  icon: (\{[\s\S]*?\n  \}),/)?.[1];
  if (!literal) continue;
  const driver = source.match(/driverKind: ProviderDriverKind.make\("([^"]+)"\)/)?.[1];
  if (!driver) throw new Error(`Missing driver for ${relative}`);
  const constants = [...source.matchAll(/^const [A-Za-z_]+ =\s*(?:"[^"\n]*"|\{[^\n]*\});/gm)].map(match => match[0]).join("\n");
  const icon = Function(`${constants}\nreturn (${literal})`)();
  for (const theme of ["light", "dark"]) {
    let defs = "";
    const paths = icon.paths.map((item, index) => {
      let fill = item.fill?.[theme] ?? icon.fill[theme];
      if (item.gradient) {
        const gradient = item.gradient;
        defs += `<linearGradient id="gradient${index}" x1="${gradient.x1}" y1="${gradient.y1}" x2="${gradient.x2}" y2="${gradient.y2}" gradientUnits="userSpaceOnUse">${gradient.stops.map(stop => `<stop offset="${escape(stop.offset)}" stop-color="${escape(stop.color)}"/>`).join("")}</linearGradient>`;
        fill = `url(#gradient${index})`;
      }
      return `<path d="${escape(item.d)}" fill="${escape(fill)}"${item.fillRule ? ` fill-rule="${escape(item.fillRule)}"` : ""}/>`;
    }).join("");
    const filename = `${driver}-${theme}.svg`;
    fs.writeFileSync(path.join(destination, filename), `<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="${icon.viewBox}"><defs>${defs}</defs>${paths}</svg>\n`);
    generated.push(filename);
  }
}
const icons = read("apps/web/src/components/Icons.tsx");
for (const [name, driver, color] of [["OpenAI", "codex"], ["ClaudeAI", "claudeAgent", "#d97757"], ["AntigravityIcon", "antigravity", "#5b87bf"], ["AnthropicIcon", "anthropic"], ["OllamaIcon", "ollama"], ["OpenRouterIcon", "openrouter"], ["LinuxIcon", "linux"]]) {
  const block = icons.match(new RegExp(`export const ${name}: Icon =[\\s\\S]*?\\n\\);`))?.[0];
  if (!block) throw new Error(`Missing original icon ${name}`);
  let svg = block.match(/<svg[\s\S]*?<\/svg>/)?.[0];
  if (!svg) throw new Error(`Missing SVG for ${name}`);
  const dataUrl = icons.match(/const ANTIGRAVITY_ICON_DATA_URL\s*=\s*("[^"]+")/)?.[1];
  if (dataUrl) svg = svg.replace("{ANTIGRAVITY_ICON_DATA_URL}", dataUrl);
  const classFill = /className=\{cn\("fill-/.test(svg);
  svg = svg.replace(/\{\.\.\.props\}/g, "").replace(/className=\{[^}]*\}/g, "").replace(/\{\/\*[\s\S]*?\*\/\}/g, "");
  if (classFill) svg = /\bfill=/.test(svg.split(">")[0]) ? svg.replace('fill="none"', 'fill="currentColor"') : svg.replace("<svg", '<svg fill="currentColor"');
  svg = svg.replace(/<svg[^>]*>/, tag => tag.replace(/\s(?:width|height)="[^"]*"/g, "").replace("<svg", '<svg width="128" height="128"'));
  for (const [jsx, xml] of [["fillRule", "fill-rule"], ["clipRule", "clip-rule"], ["strokeWidth", "stroke-width"], ["strokeLinecap", "stroke-linecap"], ["strokeLinejoin", "stroke-linejoin"]]) svg = svg.replaceAll(jsx, xml);
  if (/\{/.test(svg)) throw new Error(`Unexpected dynamic SVG for ${name}`);
  svg = svg.split("\n").map(line => line.trimEnd()).join("\n");
  for (const theme of ["light", "dark"]) {
    const filename = `${driver}-${theme}.svg`;
    fs.writeFileSync(path.join(destination, filename), svg.replaceAll("currentColor", color ?? (theme === "dark" ? "#ffffff" : "#000000")) + "\n");
    generated.push(filename);
  }
}
const machines = read("apps/web/src/components/EnvironmentMachineIcon.tsx");
for (const [name, driver] of [["MacMiniIcon", "mac-mini"], ["MacStudioIcon", "mac-studio"]]) {
  const block = machines.match(new RegExp(`function ${name}\\([\\s\\S]*?\\n\\}`))?.[0];
  const body = block?.match(/<LucideLike[^>]*>([\s\S]*?)<\/LucideLike>/)?.[1];
  if (!body) throw new Error(`Missing original machine icon ${name}`);
  const filename = `${driver}.svg`;
  fs.writeFileSync(path.join(destination, filename), `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">${body}</svg>\n`);
  generated.push(filename);
}
fs.writeFileSync(path.join(destination, "sources.json"), JSON.stringify({ sources, generated }, null, 2) + "\n");
fs.writeFileSync(path.join(destination, "assets.rs"), "pub const ASSETS: &[(&str, &[u8])] = &[\n" + generated.map(filename => `    ("icons/providers/${filename}", include_bytes!("${filename}")),`).join("\n") + "\n];\n");
console.log(JSON.stringify({ generated: generated.length }));
