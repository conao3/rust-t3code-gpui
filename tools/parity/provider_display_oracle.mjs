import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { stripTypeScriptTypes } from "node:module";
import { boundaries } from "./boundaries.mjs";

const [root, destination] = process.argv.slice(2);
if (!root || !destination) throw new Error("Expected original source root and destination");
const paths = {
  display: "packages/client-runtime/src/state/providerInstanceDisplay.ts",
  models: "packages/client-runtime/src/state/models.ts",
  contracts: "packages/contracts/src/model.ts",
  instance: "packages/contracts/src/providerInstance.ts",
  entries: "apps/web/src/providerInstances.ts",
  server: "packages/contracts/src/server.ts",
  sort: "packages/client-runtime/src/state/threadSort.ts",
  timestamp: "apps/web/src/timestampFormat.ts",
  row: "apps/web/src/components/sidebar/SidebarThreadRow.tsx",
};
const sources = Object.fromEntries(Object.entries(paths).map(([key, value]) => [key, fs.readFileSync(path.join(root, value), "utf8")]));
const fn = (source, name) => {
  const value = source.match(new RegExp(`(?:export )?function ${name}\\([\\s\\S]*?\\n\\}`))?.[0];
  if (!value) throw new Error(`Missing original function ${name}`);
  return value.startsWith("export ") ? value : "export " + value;
};
const required = (source, expression) => {
  const value = source.match(expression)?.[0];
  if (!value) throw new Error(`Missing source declaration ${expression}`);
  return value;
};
const branding = "const ProviderDriverKind={make:value=>value}; const ProviderInstanceId={make:value=>value};\n";
const code = [
  branding,
  ...sources.contracts.matchAll(/^const [A-Z_]+_DRIVER_KIND = ProviderDriverKind.make\("[^"\n]+"\);/gm).map(match => match[0]),
  required(sources.contracts, /export const PROVIDER_DISPLAY_NAMES:[\s\S]*?\n\};/),
  required(sources.instance, /export const defaultInstanceIdForDriver =[\s\S]*?;/),
  required(sources.models, /const THREAD_PROVIDER_STACK_LIMIT = \d+;/),
  ...["humanizeSlug", "resolveProviderInstanceDisplayName", "providerInstanceInitials", "normalizeProviderAccentColor", "providerAccentForegroundColor", "shouldShowInstanceBadge"].map(name => fn(sources.display, name)),
  fn(sources.models, "resolveThreadProviderStack"),
  ...["deriveProviderInstanceEntries", "brandProviderInstanceEntries", "applyProviderInstanceBranding"].map(name => fn(sources.entries, name)),
  fn(sources.server, "resolveEnvironmentMachineKind"),
  ...["toSortableTimestamp", "resolveThreadActivityTimestamp"].map(name => fn(sources.sort, name)),
  fn(sources.timestamp, "formatRelativeElapsed"),
  fn(sources.row, "compactSidebarTimeLabel"),
].join("\n");
const original = await import("data:text/javascript;base64," + Buffer.from(stripTypeScriptTypes(code)).toString("base64"));
const initials = ["Codex", "Codex (hub)", "Theo's Codex", "Codex 'hub'", "Codex ’hub’", "'", "’", "'',", "( )", "", "A", "ß", "ßeta test", "🙂🙂🙂", "日本語", "日本 語", "Ｃｏｄｅｘ hub", "foo_bar", "foo/bar", "foo—bar", "foo…bar", "a\u0085b", "a\uFEFFb", "a\u3000b", "a\u00A0b", "a\u2028b", "'foo' 'bar'", "‘foo’ ‘bar’", "_a_", "İstanbul Server", "i\u0307 server", "🤖 Codex"].map(input => ({input, expected: original.providerInstanceInitials(input)}));
const names = [];
names.push(...boundaries([["codex", "claudeAgent", "opencode", "acpRegistry", "antigravity", "cursor", "grok", "fx", "kiro", "muse", "pi", "openaiCompatible", "customDriver"], [driver => driver, driver => driver + "_conao3", () => "東京_worker", () => "aBCd--test", () => "\uFEFFname\uFEFF"], [undefined, "", "  ", "Codex", "Claude", "東京 用", "\uFEFFcustom\uFEFF"]], ([driver, instance, displayName]) => {
  const input = {driver, instanceId: instance(driver), ...(displayName === undefined ? {} : {displayName})};
  return {input, expected: original.resolveProviderInstanceDisplayName(input)};
}));
const accents = ["", " ", "#000000", "#ffffff", "#Aa00Ff", "\uFEFF#123456\uFEFF", "#123", "#1234567", "#12zz34", "red", "#日本語", "\u0085#123456"].map(input => ({input, expected: original.normalizeProviderAccentColor(input) ?? null}));
const contrast = [];
contrast.push(...boundaries([[0, 64, 128, 192, 255], [0, 64, 128, 192, 255], [0, 64, 128, 192, 255]], ([r, g, b]) => {
  const input = "#" + [r,g,b].map(channel => channel.toString(16).padStart(2,"0")).join("");
  return {input, expected: original.providerAccentForegroundColor(input)};}));
const stacks = [];
for (const runtime of [undefined, null, {providerInstanceId:"a"}, {providerInstanceId:"b"}]) {
  for (const instanceId of ["a", "b"]) for (const history of [[], ["a"], ["b"], ["a","b"], ["b","a"], ["a","a","b"], ["a","b","c","d"], ["a","b","a","c","b","d"]]) {
    const input = {modelSelection:{instanceId}, providerInstanceHistory:history, ...(runtime === undefined ? {} : {runtime})};
    stacks.push({input, expected: original.resolveThreadProviderStack(input)});
  }
}
const provider = (instanceId, driver = "codex", extra = {}) => ({instanceId, driver, enabled:true, installed:true, status:"ready", models:[], ...extra});
const catalogs = [];
const addCatalog = (providers, providerInstances = {}, machine = "desktop", environmentIcon) => {
  const input = {providers, settings:{providerInstances, ...(environmentIcon === undefined ? {} : {environmentIcon})}, environment:{platform:{machine}}};
  const entries = original.brandProviderInstanceEntries(original.deriveProviderInstanceEntries(providers), input.settings);
  catalogs.push({input, machine:original.resolveEnvironmentMachineKind(input), entries:entries.map(entry => ({
    instance_id:entry.instanceId, driver:entry.driverKind, name:entry.displayName, enabled:entry.enabled,
    accent:entry.accentColor ?? null, registry_agent:entry.acpRegistryAgentId ?? null, registry_icon:entry.acpRegistryIconUrl ?? null,
    badge:original.shouldShowInstanceBadge(entry, entries),
  }))});
};
addCatalog([]);
for (const enabled of [false,true]) for (const otherEnabled of [false,true]) for (const accentColor of [undefined,"#445566","invalid"]) {
  addCatalog([provider("codex","codex",{enabled,accentColor}),provider("codex_second","codex",{enabled:otherEnabled})]);
}
addCatalog([provider("a","codex"),provider("b","claudeAgent"),provider("c","opencode")]);
for (const source of [undefined,"local","registry"]) for (const agentId of [undefined,"same","other"]) {
  addCatalog([provider("a","acpRegistry",{iconUrl:"https://example.invalid/a.svg"}),provider("b","acpRegistry")],{a:{config:{source,agentId,registryIconUrl:"  https://example.invalid/override.svg  "}},b:{config:{agentId:"same"}}});
}
for (const machine of ["server","cloud","linux","desktop","laptop","mac-mini","mac-studio"]) for (const setting of [undefined,null,"desktop","laptop"]) addCatalog([],{},machine,setting);
const times = [-60000,0,59999,60000,3599999,3600000,86399999,86400000,172800000,31536000000].map(elapsed => {
  const now = Date.parse("2026-11-17T12:00:00.000Z");
  const timestamp = new Date(now - elapsed).toISOString();
  const parts = original.formatRelativeElapsed(Math.max(0, elapsed));
  return {now, timestamp, expected: original.compactSidebarTimeLabel(parts.value + (parts.suffix ? " " + parts.suffix : ""))};
});
const activity = [];
for (const updatedAt of [undefined, "invalid", "2026-11-17T00:00:00Z"]) for (const latestUserMessageAt of [undefined, "invalid", "2026-11-17T03:00:00Z"]) for (const latestRun of [undefined, {requestedAt:"2026-11-17T02:00:00Z"}, {requestedAt:"2026-11-17T01:00:00Z",startedAt:"2026-11-17T04:00:00Z",completedAt:"2026-11-17T05:00:00Z"}, {completedAt:"invalid"}]) {
  const input={updatedAt,latestUserMessageAt,latestRun};
  activity.push({input,expected:original.resolveThreadActivityTimestamp(input)});
}
fs.writeFileSync(destination,JSON.stringify({sourceRevision:"24e77473ecf41bc1f7142529c7911c1c1aae1d76",sources:Object.fromEntries(Object.entries(paths).map(([key,value])=>[value,createHash("sha256").update(sources[key]).digest("hex")])),adapter:"Contract string brands are identity functions; original display, metadata, badge, history and machine functions execute unchanged",initials,names,accents,contrast,stacks,catalogs,times,activity},null,2)+"\n");
console.log(JSON.stringify({initials:initials.length,names:names.length,accents:accents.length,contrast:contrast.length,stacks:stacks.length,catalogs:catalogs.length}));
