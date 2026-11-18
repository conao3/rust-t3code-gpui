import fs from "node:fs";
import path from "node:path";
import {createHash} from "node:crypto";
import {stripTypeScriptTypes} from "node:module";
import {boundaries} from "./boundaries.mjs";
const [root,destination]=process.argv.slice(2);
if (!root||!destination) throw new Error("Expected original root and destination");
const file="apps/web/src/components/Sidebar.logic.ts";
const source=fs.readFileSync(path.join(root,file),"utf8");
const names=["resolveSidebarThreadStatus","shouldRecedeSidebarThread","hasUnseenCompletion"];
const functions=names.map(name=> {
 const code=source.match(new RegExp(`export function ${name}\\([\\s\\S]*?\\n\\}(?=\\r?\\n|$)`))?.[0];
 if (!code) throw new Error(`Missing ${name}`);
 return code;
});
const modelsFile="packages/client-runtime/src/state/models.ts";
const models=fs.readFileSync(path.join(root,modelsFile),"utf8");
for (const name of ["resolveThreadWorkingStartedAt","formatWorkingDurationLabel"]) {
 const code=models.match(new RegExp(`export function ${name}\\([\\s\\S]*?\\n\\}(?=\\r?\\n|$)`))?.[0];
 if (!code) throw new Error(`Missing ${name}`);
 functions.push(code);
}
const original=await import("data:text/javascript;base64,"+Buffer.from(stripTypeScriptTypes(functions.join("\n"))).toString("base64"));
const statuses=[];
statuses.push(...boundaries([[false,true], [false,true], [null,"approval","user_input"], [0,1], [null,"preparing","queued","starting","running","waiting","idle","failed","stopped"], [false,true], [null,"usage_limit"]], ([hasPendingApprovals, hasPendingUserInput, pendingSubagentRequest, pendingGuestMessageCount, status, blockedOnThreadWait, lastErrorClass]) => {
 const input={hasPendingApprovals,hasPendingUserInput,pendingSubagentRequest,pendingGuestMessageCount,runtime:status===null?null:{status,lastErrorClass},blockedOnThreadWait};
 return {input,expected:original.resolveSidebarThreadStatus(input)};}));
const receding=[];
receding.push(...boundaries([["approval","input","request","preparing","working","waiting","failed","limited","ready"], [false,true], [false,true], [false,true], [false,true]], ([status, isUnread, isWoke, isActive, isSelected]) => {
 const input={status,isUnread,isWoke,isActive,isSelected};
 return {input,expected:original.shouldRecedeSidebarThread(input)};}));
const unread=[];
for (const completedAt of [null,"", "invalid","2026-11-17T10:00:00.000Z","2026-11-17T10:00:00+01:00"]) for (const lastVisitedAt of [null,"","invalid","2026-11-17T09:00:00.000Z","2026-11-17T10:00:00.000Z","2026-11-17T11:00:00.000Z"]) {
 const input={latestRun:completedAt===null?null:{completedAt},lastVisitedAt};
 unread.push({input,expected:original.hasUnseenCompletion(input)});
}
const starts=[];
starts.push(...boundaries([[undefined,null,"invalid","2026-11-17T10:00:00.000Z"], [null,"run","another"], [undefined,null,"2026-11-17T11:00:00.000Z"], [null,"invalid","2026-11-17T10:00:00.000Z"]], ([activityStartedAt, activeRunId, completedAt, startedAt]) => {
 const input={runtime:{activeRunId,...(activityStartedAt===undefined?{}:{activityStartedAt})},latestRun:{runId:"run",startedAt,requestedAt:"2026-11-17T09:00:00.000Z",...(completedAt===undefined?{}:{completedAt})}};
 return {input,expected:original.resolveThreadWorkingStartedAt(input)};}));
const durations=[-1000,0,999,1000,59000,59999,60000,3599999,3600000,3660000,86400000].map(input=>({input,expected:original.formatWorkingDurationLabel(input)}));
fs.writeFileSync(destination,JSON.stringify({sourceRevision:"24e77473ecf41bc1f7142529c7911c1c1aae1d76",source:{path:file,sha256:createHash("sha256").update(source).digest("hex")},modelsSource:{path:modelsFile,sha256:createHash("sha256").update(models).digest("hex")},statuses,receding,unread,starts,durations},null,2)+"\n");
console.log(JSON.stringify({statuses:statuses.length,receding:receding.length,unread:unread.length,starts:starts.length,durations:durations.length}));
