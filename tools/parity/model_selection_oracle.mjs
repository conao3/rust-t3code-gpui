import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { stripTypeScriptTypes } from "node:module";
import { boundaries, unique } from "./boundaries.mjs";

const [root, destination] = process.argv.slice(2);
if (!root || !destination) throw new Error("Expected reference root and destination");
const paths = {
  settings: "packages/contracts/src/settings.ts",
  contracts: "packages/contracts/src/model.ts",
  shared: "packages/shared/src/model.ts",
  selection: "apps/web/src/modelSelection.ts",
  ordering: "apps/web/src/modelOrdering.ts",
  actions: "apps/web/src/lib/chatThreadActions.ts",
};
const source = Object.fromEntries(Object.entries(paths).map(([key, file]) => [key, fs.readFileSync(path.join(root, file), "utf8")]));
const required = (source, expression) => {
  const match = source.match(expression)?.[0];
  if (!match) throw new Error(`Missing original declaration ${expression}`);
  return match;
};
const fn = (source, name) => required(source, new RegExp(`(?:export )?function ${name}(?:<[^]*?>)?\\([^]*?\\n\\}(?=\\n|$)`)).replace(/^function /, "export function ");
const adapters = `
const ProviderDriverKind = {make: value => value};
const ProviderInstanceId = {make: value => value};
const DEFAULT_PROVIDER_DRIVER_KIND = "codex";
const MAX_CUSTOM_MODEL_LENGTH = 256;
const ANTIGRAVITY_DEFAULT_MODEL = "antigravity-default";
const Option = {getOrNull: value => value};
const decodeCustomModelCapabilities = value => value;
const createModelCapabilities = value => value;
const Result = {failVoid: null, succeed: value => ({value})};
const Arr = {map: (values, fn) => values.map(fn), filterMap: (values, fn) => values.flatMap(value => {const result = fn(value); return result === null ? [] : [result.value];}), sort: (values, order) => [...values].sort(order)};
const Order = {Number: (a,b) => a === b ? 0 : a < b ? -1 : 1, Boolean: (a,b) => Number(a) - Number(b), mapInput: (order, fn) => (a,b) => order(fn(a), fn(b)), flip: order => (a,b) => order(b,a), combineAll: orders => (a,b) => {for (const order of orders) {const result = order(a,b); if(result) return result;} return 0;}};
const resolveModelShortName = model => model.shortName;
const deriveProviderInstanceEntries = providers => providers.map(provider => ({...provider, driverKind: provider.driver}));
`;
const code = [
  adapters,
  ...source.contracts.matchAll(/^const [A-Z_]+_DRIVER_KIND = ProviderDriverKind.make\("[^"\n]+"\);/gm).map(match => match[0]),
  required(source.contracts, /export const MODEL_SLUG_ALIASES_BY_PROVIDER:[^]*?\n\};/),
  required(source.settings, /const DEFAULT_DISABLED_PROVIDER_DRIVERS:[^]*?\n\);/),
  required(source.settings, /const DEFAULT_ENABLED_DEFAULT_INSTANCES:[^]*?\n\);/),
  required(source.settings, /export const providerInstanceConfigEnabledFlag =[^]*?\n\};/),
  required(source.settings, /export const isUnconfiguredDefaultInstanceEnabled =[^]*?;/),
  required(source.settings, /const defaultEnabledForDriver =[^]*?;/),
  required(source.settings, /export const resolveProviderInstanceEnabled =[^]*?\n\};/),
  required(source.settings, /export const resolveProjectProviderInstanceEnabled =[^]*?\n\};/),
  ...["normalizeCustomModelSlug", "normalizeModelSlug", "readCustomModelEntries", "resolveSelectableModel"].map(name => fn(source.shared, name)),
  ...["rankByValue", "toSet", "byOptionalRank", "byTrueFirst", "sortModelsForProviderInstance"].map(name => fn(source.ordering, name)),
  ...["readInstanceCustomModels", "appendUnavailableDynamicModelSelection", "toAppModelOption", "readInstanceModelPreferences", "applyInstanceModelPreferences", "normalizeCustomModelEntries", "getAppModelOptionsForInstance", "resolveAppModelSelectionForInstance", "resolveEffectiveDefaultModelSelection"].map(name => fn(source.selection, name)),
  fn(source.actions, "resolveNewThreadModelSelectionOverride"),
].join("\n");
const original = await import("data:text/javascript;base64," + Buffer.from(stripTypeScriptTypes(code)).toString("base64"));
const enabled = boundaries([["codex", "claudeAgent", "cursor", "opencode", "antigravity", "acpRegistry"], [false, true], [false, true], [undefined, false, true], [undefined, false, true], [undefined, false, true]], ([driver, custom, configured, top, nested, override]) => {
  const instance = custom ? driver + "_custom" : driver;
  const settings = {providerInstances: configured ? {[instance]: {driver, enabled: top, config: {enabled: nested}}} : {}, projectSettingsOverrides: {project: {providerInstances: {[instance]: {enabled: override}}}}};
  return {settings, project: "project", instance, expected: original.resolveProjectProviderInstanceEnabled(settings, "project", instance)};
});
const selections = boundaries([["codex", "opencode", "antigravity", "cursor"], [[], ["gpt-5.4"], ["custom"], ["gpt-5.4", "second", "unknown"], ["gpt-5.4", "second"]], [[], [" custom ", {slug:"named", name:"My model"}, "custom", "gpt-5.4"], [{slug:"named", name:"My model"}], ["x".repeat(257)]], ["gpt-5.4"," 5.4 ","GPT-5.4","ALIAS","custom","My model","unknown","antigravity-default","stale-custom",""]], ([driver, hidden, custom, model]) => {
  const providers = [{instanceId:driver, driver, enabled:true, status:"ready", models:[{slug:"gpt-5.4",name:"GPT-5.4",isDefault:true,aliases:["alias"],isCustom:false},{slug:"second",name:"Second",isCustom:false},{slug:"stale-custom",name:"Stale",isCustom:true}]}];
  const settings = {providerInstances:{[driver]:{driver,config:{customModels:custom}}}, providerModelPreferences:{[driver]:{hiddenModels:hidden, modelOrder:["second","gpt-5.4"]}}};
  const selection = {instanceId:driver,model,options:[{id:"effort",value:"high"}]};
  return {config:{settings,providers}, selection, expected: original.resolveEffectiveDefaultModelSelection(settings, providers, selection)};
});
const overrides = [];
for (const projectDefaultSelection of [null,{instanceId:"codex",model:"project"}]) for (const carrySelection of [null,{instanceId:"codex",model:"carry"}]) for (const carrySourceDraftId of [null,"same","other"]) {
  const input = {projectDefaultSelection,carrySelection,carrySourceDraftId,destinationDraftId:"same"};
  overrides.push({input,expected:original.resolveNewThreadModelSelectionOverride(input)});
}
fs.writeFileSync(destination, JSON.stringify({sources:Object.fromEntries(Object.entries(paths).map(([key,file])=>[file,createHash("sha256").update(source[key]).digest("hex")])),adapters:["Effect Array/Order/Result operations", "Brand identity", "Entry metadata and short name projection", "Custom capability identity for fixtures without capabilities"],enabled:unique(enabled),selections,overrides},null,2)+"\n");
console.log(JSON.stringify({enabled:unique(enabled).length,selections:selections.length,overrides:overrides.length}));
