import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { stripTypeScriptTypes } from "node:module";
import { pairwise } from "./pairwise.mjs";

const [root, destination] = process.argv.slice(2);
if (!root || !destination) throw new Error("Expected original source root and destination");
const paths = {
  models: "packages/client-runtime/src/state/models.ts",
  background: "packages/shared/src/orchestrationV2PendingBackgroundWork.ts",
  sort: "packages/client-runtime/src/state/threadSort.ts",
  settled: "packages/client-runtime/src/state/threadSettled.ts",
  settings: "packages/contracts/src/settings.ts",
};
const sources = Object.fromEntries(Object.entries(paths).map(([name, file]) => [name, fs.readFileSync(path.join(root, file), "utf8")]));
const fn = (source, name) => {
  const value = source.match(new RegExp(`(?:export )?function ${name}\\([\\s\\S]*?\\n\\}`))?.[0];
  if (!value) throw new Error(`Missing original function ${name}`);
  return value.startsWith("export ") ? value : "export " + value;
};
const adapter = `const DateTime={formatIso:value=>new Date(value).toISOString()};
const threadPullRequestsOf=()=>[];
const formatSubagentDisplayTitle=value=>value;
const THREAD_PROVIDER_STACK_LIMIT=3;`;
const code = [adapter,
  ...["backgroundWorkKindHoldsCompletion", "backgroundWorkHoldsCompletion"].map(name => fn(sources.background, name)),
  ...["iso", "nullableIso", "terminalRunStatus", "shellRuntime", "presentThreadShell", "resolveThreadProviderStack"].map(name => fn(sources.models, name)),
  ...["toSortableTimestamp", "resolveThreadActivityTimestamp"].map(name => fn(sources.sort, name)),
].join("\n");
const original = await import("data:text/javascript;base64," + Buffer.from(stripTypeScriptTypes(code)).toString("base64"));
const keys = ["providerInstanceId", "id", "createdAt", "archivedAt", "unsettledAt", "pinnedAt", "pinOrderKey", "activeOrderKey", "interactionMode", "hasActionableProposedPlan", "latestUserAuthoredMessageAt", "runtime", "latestRun", "hasPendingApprovals", "hasPendingUserInput", "pendingUserInputAt", "pendingSubagentRequest", "pendingGuestMessageCount", "latestUserMessageAt", "snoozedUntil", "snoozedAt", "settledAt", "settledOverride", "goal", "blockedOnThreadWait", "providerInstanceHistory", "modelSelection", "updatedAt", "lastVisitedAt"];
const base = {
  interactionMode:"default",hasActionableProposedPlan:false,pinnedAt:null,pinOrderKey:null,activeOrderKey:null,unsettledAt:null,id:"fixture",projectId:"project",title:"Fixture",createdAt:"2026-11-17T00:00:00.000Z",updatedAt:"2026-11-17T12:00:00.000Z",
  lineage:{relationshipToParent:"root"},status:"idle",latestRunId:null,activeRunId:null,activeProviderThreadId:null,
  providerInstanceId:"previous",modelSelection:{instanceId:"next",model:"fixture"},pendingRuntimeRequest:null,
  latestUserMessageAt:"2026-11-17T10:00:00.000Z",archivedAt:null,deletedAt:null,settledAt:null,snoozedUntil:null,snoozedAt:null,
  providerInstanceHistory:["a","b","previous"],
};
const presentation = [];
const add = (extra, pendingSubagent=null) => {
  const input={...base,...extra};
  const state=original.presentThreadShell("environment",input,pendingSubagent);
  const expected=Object.fromEntries(keys.filter(key=>state[key]!==undefined).map(key=>[key,state[key]]));
  expected.source={pendingRuntimeRequest:input.pendingRuntimeRequest};
  presentation.push({input,pendingSubagent,expected,stack:original.resolveThreadProviderStack(state),activity:original.resolveThreadActivityTimestamp(state)});
};
for (const [status,ids,pendingBackgroundTasks] of pairwise([["idle","pending","running","waiting","completed","failed","cancelled","interrupted","rolled_back"],[{latestRunId:null,activeProviderThreadId:null},{latestRunId:"run",activeProviderThreadId:null},{latestRunId:null,activeProviderThreadId:"provider-thread"}],[[],[{kind:"command",awaited:false}],[{kind:"command",awaited:true}],[{kind:"subagent",awaited:false}],[{kind:"monitor",awaited:false}],[{kind:"background_task",awaited:false}]]])) add({status,...ids,pendingBackgroundTasks});
for (const status of ["idle","running","failed"]) for (const latestRunCompletedAt of [undefined,null,"2026-11-17T11:00:00.000Z"]) {
  add({status,latestRunId:"run",latestRunRequestedAt:"2026-11-17T10:30:00.000Z",latestRunStartedAt:"2026-11-17T10:31:00.000Z",...(latestRunCompletedAt===undefined?{}:{latestRunCompletedAt})});
}
for (const kind of ["user_input","auth_refresh","approval","command"]) for (const pending of [null,"approval","user_input"]) {
  add({pendingRuntimeRequest:{kind,createdAt:"2026-11-17T11:00:00.000Z"},pendingGuestMessageCount:2},pending);
}
for (const activityRunStatus of [undefined,null,"idle","running","error"]) for (const activityRunStartedAt of [undefined,null,"2026-11-17T10:59:00.000Z"]) {
  add({status:"waiting",latestRunId:"run",...(activityRunStatus===undefined?{}:{activityRunStatus}),...(activityRunStartedAt===undefined?{}:{activityRunStartedAt})});
}
add({latestRunId:"run",updatedAt:"2026-11-17T07:00:00-05:00",lastVisitedAt:"2026-11-17T06:00:00-05:00",latestRunStartedAt:"2026-11-17T07:30:00-05:00"});
for (const settledOverride of [null,"active","settled"]) for (const blockedOnThreadWait of [false,true]) for (const goal of [null,{status:"active"}]) add({settledOverride,blockedOnThreadWait,goal});
for (const latestUserAuthoredMessageAt of [undefined,null,"2026-11-17T09:00:00.000Z"]) for (const interactionMode of ["default","plan"]) for (const hasActionableProposedPlan of [false,true]) add({interactionMode,hasActionableProposedPlan,...(latestUserAuthoredMessageAt===undefined?{}:{latestUserAuthoredMessageAt})});
const negativeRawWiring = {stack:presentation.filter(value=>JSON.stringify(original.resolveThreadProviderStack(value.input))!==JSON.stringify(value.stack)).length,activity:presentation.filter(value=>original.resolveThreadActivityTimestamp(value.input)!==value.activity).length};
fs.writeFileSync(destination,JSON.stringify({sourceRevision:"24e77473ecf41bc1f7142529c7911c1c1aae1d76",sources:Object.fromEntries(Object.entries(paths).map(([name,file])=>[file,createHash("sha256").update(sources[name]).digest("hex")])),adapter:"UTC Effect dates use JS Date conversion. Empty PR lists and root titles have identity adapters; neither is an asserted field. Original presentation, background hold, provider stack and activity functions execute unchanged.",negativeRawWiring,presentation},null,2)+"\n");
console.log(JSON.stringify({presentation:presentation.length,negativeRawWiring}));
