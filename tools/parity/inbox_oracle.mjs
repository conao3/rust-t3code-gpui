import fs from "node:fs";
import path from "node:path";
import {createHash} from "node:crypto";
import {stripTypeScriptTypes} from "node:module";
import {boundaries,unique} from "./boundaries.mjs";
const [root,destination]=process.argv.slice(2);
if (!root||!destination) throw new Error("Expected original root and destination");
const paths={inbox:"packages/client-runtime/src/state/threadInbox.ts",models:"packages/client-runtime/src/state/models.ts",settled:"packages/client-runtime/src/state/threadSettled.ts",sort:"packages/client-runtime/src/state/threadSort.ts",sidebar:"apps/web/src/components/Sidebar.tsx",logic:"apps/web/src/components/Sidebar.logic.ts"};
const sources=Object.fromEntries(Object.entries(paths).map(([name,file])=>[name,fs.readFileSync(path.join(root,file),"utf8")]));
const extract=(file,name)=>{
 const value=sources[file].match(new RegExp(`(?:export )?function ${name}(?:<[\\s\\S]*?>)?\\([\\s\\S]*?\\n\\}(?=\\r?\\n|$)`))?.[0];
 if (!value) throw new Error(`Missing ${file}:${name}`);
 return value.startsWith("export ")?value:"export "+value;
};
const functions=[...['threadRunStatusIsActive','threadRuntimeIsActive'].map(name=>extract('models',name)),...['pendingQuestionAt','failureEdgeAt','threadRaisedHandWhileSnoozed','effectiveSnoozed'].map(name=>extract('settled',name)),...['isThreadWorking','asLaunchingThread','createInboxReturnTracker','sortInboxThreadsByReturn','sortWorkingThreadsBySend','sortNewestFirst'].map(name=>extract('inbox',name)),...['toSortableTimestamp','resolveSettledThreadTimestamp','resolveThreadActivityTimestamp','sortSettledThreads','activeThreadAnchorTimestampMs','sortPinnedThreadsByOrderKey','sortActiveThreadsByOrderKey'].map(name=>extract('sort',name))];
const snoozedExpression=sources.sidebar.match(/snoozedThreads: (snoozed\.toSorted\([\s\S]*?\n    \)),/)?.[1];
if(!snoozedExpression)throw new Error("Missing original Snoozed sorting expression");
functions.push(extract('logic','firstValidTimestampMs'),`export function sortSnoozed(snoozed){return ${snoozedExpression};}`);
const original=await import('data:text/javascript;base64,'+Buffer.from(stripTypeScriptTypes('const WORKING_EXIT_GRACE_MS=2000;const DateTime={formatIso:value=>new Date(value).toISOString()};\n'+functions.join('\n'))).toString('base64'));
const base={providerInstanceId:'codex',updatedAt:'2026-11-17T00:00:00.000Z',id:'thread',environmentId:'environment',createdAt:'2026-11-17T00:00:00.000Z',unsettledAt:null,interactionMode:'default',hasActionableProposedPlan:false,hasPendingApprovals:false,hasPendingUserInput:false,pendingSubagentRequest:null,pendingGuestMessageCount:0,runtime:{status:'running',activeRunId:'run',lastError:null},latestRun:{runId:'run',status:'running'},archivedAt:null,deletedAt:null,pinnedAt:null,settledOverride:'active',settledAt:null,snoozedAt:null,snoozedUntil:null,activeOrderKey:null};
const working=[];
working.push(...boundaries([[null,'preparing','queued','starting','running','waiting','idle','failed','interrupted'], [null,'preparing','queued','starting','running','waiting','completed','failed'], [null,'run'], ['default','plan'], [false,true]], ([status, runStatus, activeRunId, interactionMode, hasActionableProposedPlan]) => {
 const input={...base,runtime:status===null?null:{status,activeRunId},latestRun:runStatus===null?null:{runId:'run',status:runStatus},interactionMode,hasActionableProposedPlan};
 return {input,expected:original.isThreadWorking(input)};}, value=>[value.expected,original.asLaunchingThread(value.input)!==value.input]));
working.push(...boundaries([[false,true], [false,true], [null,'approval','user_input'], [0,1]], ([hasPendingApprovals, hasPendingUserInput, pendingSubagentRequest, pendingGuestMessageCount]) => {
 const input={...base,hasPendingApprovals,hasPendingUserInput,pendingSubagentRequest,pendingGuestMessageCount};
 return {input,expected:original.isThreadWorking(input)};}, value=>[value.expected,original.asLaunchingThread(value.input)!==value.input]));
const RealDate=globalThis.Date;
let clock=0,sequence=0;const timers=new Map();
globalThis.Date=class extends RealDate {constructor(...args){super(...(args.length?args:[clock]));}static now(){return clock;}};
globalThis.setTimeout=(callback,delay)=>{const id=++sequence;timers.set(id,{callback,at:clock+delay});return id;};
globalThis.clearTimeout=id=>timers.delete(id);
const finished={...base,runtime:null,latestRun:{runId:'run',status:'completed'}};
const scenarios=[];
function scenario(name,steps){
 timers.clear();clock=0;const tracker=original.createInboxReturnTracker();let threads=[];const results=[];
 for (const step of steps){
  const at=step.at;
  if (step.action==='advance'){
   while (true){const due=[...timers].filter(([,timer])=>timer.at<=at).sort((a,b)=>a[1].at-b[1].at)[0];if (!due)break;clock=step.late?at:due[1].at;timers.delete(due[0]);due[1].callback();}
   clock=at;
  }else{clock=at;threads=step.threads;tracker.observe(threads);}
  results.push({...step,expected:{version:tracker.getSnapshot(),deadline:timers.size?Math.min(...[...timers.values()].map(timer=>timer.at)):null,threads:(threads??[]).map(thread=>({id:thread.id,environmentId:thread.environmentId,working:tracker.isWorking(thread),returnedAt:tracker.returnedAt(thread)??null}))}});
 }
 scenarios.push({name,steps:results});
}
const observe=(at,threads)=>({action:'observe',at,threads});const advance=(at,late=false)=>({action:'advance',at,late});
scenario('grace-stable-and-expiry',[observe(0,[base]),observe(1000,[finished]),advance(2999),observe(2999,[finished]),advance(3000),observe(3100,[finished]),observe(4000,[base]),observe(5000,[finished]),advance(9000,true)]);
scenario('continuation',[observe(0,[base]),observe(1000,[finished]),observe(2000,[base]),advance(5000),observe(6000,[finished]),advance(8000)]);
for(const [field,value] of Object.entries({archivedAt:'2026-11-17T00:00:00.000Z',deletedAt:'2026-11-17T00:00:00.000Z',pinnedAt:'2026-11-17T00:00:00.000Z',settledOverride:'settled',settledAt:'2026-11-17T00:00:00.000Z',snoozedAt:'2026-11-17T00:00:00.000Z',snoozedUntil:'2026-11-17T01:00:00.000Z',activeOrderKey:'ab'}))scenario('move-'+field,[observe(0,[base]),observe(1000,[finished]),observe(1500,[{...finished,[field]:value}]),advance(4000)]);
for(const attention of [{hasPendingApprovals:true},{hasPendingUserInput:true},{pendingSubagentRequest:'approval'},{pendingGuestMessageCount:1},{interactionMode:'plan',hasActionableProposedPlan:true},{runtime:{status:'failed',lastError:'failed'}},{latestRun:{runId:'run',status:'failed'}}])scenario('attention-'+JSON.stringify(attention),[observe(0,[base]),observe(1000,[finished]),observe(1500,[{...finished,...attention}]),advance(4000)]);
scenario('removed-disabled-and-reappearing',[observe(0,[base]),observe(1000,[finished]),advance(3000),observe(4000,[]),observe(5000,[finished]),observe(6000,[base]),observe(7000,[finished]),observe(7500,null),observe(8000,[finished]),advance(10000)]);
scenario('two-environments-independent',[observe(0,[base,{...base,environmentId:'other'}]),observe(1000,[finished,{...base,environmentId:'other'}]),advance(3000),observe(4000,[finished,{...finished,environmentId:'other'}]),advance(6000)]);
const launching=working.map(({input})=>({input,expected:original.asLaunchingThread(input)}));
globalThis.Date=RealDate;
if(new Intl.DateTimeFormat().resolvedOptions().locale!=="en-US")throw new Error("Generate the sorting oracle with the en-US default locale");
const sorting=[];let seed=81;
const next=n=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed%n;};
const dates=[undefined,null,"invalid","1960-01-01T00:00:00.000Z","2026-11-17T00:00:00.000Z","2026-11-17T10:00:00.000Z","2026-11-17T10:00:00-04:00"];
const ids=['a','A','0','a-1','aa','ä','z','😀'];
for(let iteration=0;iteration<30;iteration++){
 const input=ids.map(id=>({id,environmentId:next(2)?'env-a':'env-b',createdAt:dates[next(dates.length)],unsettledAt:dates[next(dates.length)],updatedAt:dates[next(dates.length)],settledAt:dates[next(dates.length)],latestUserMessageAt:dates[next(dates.length)],...(next(3)?{latestUserAuthoredMessageAt:dates[next(dates.length)]}:{}),latestRun:{requestedAt:dates[next(dates.length)],startedAt:dates[next(dates.length)],completedAt:dates[next(dates.length)]},pinOrderKey:[undefined,null,'','ab','ac','ad'][next(6)],activeOrderKey:[undefined,null,'','ab','ac','ad'][next(6)],snoozedUntil:dates[next(dates.length)],observedReturnAt:next(3)?undefined:Date.parse('2026-11-17T12:00:00.000Z')}));
 const identify=rows=>rows.map(thread=>[thread.environmentId,thread.id]);
 sorting.push({input,expected:{pinned:identify(original.sortPinnedThreadsByOrderKey(input)),active:identify(original.sortActiveThreadsByOrderKey(input)),inbox:identify(original.sortInboxThreadsByReturn(input,thread=>thread.observedReturnAt)),working:identify(original.sortWorkingThreadsBySend(input)),settled:identify(original.sortSettledThreads(input)),snoozed:identify(original.sortSnoozed(input))}});
}

fs.writeFileSync(destination,JSON.stringify({sourceRevision:'24e77473ecf41bc1f7142529c7911c1c1aae1d76',sources:Object.fromEntries(Object.entries(paths).map(([name,file])=>[file,createHash('sha256').update(sources[name]).digest('hex')])),adapter:'Original functions execute unchanged. A virtual wall clock and setTimeout queue run exact and delayed deadlines; undefined returnedAt is serialized as null.',working:unique(working),launching:unique(launching),scenarios,sorting},null,2)+'\n');
console.log(JSON.stringify({working:unique(working).length,sorting:sorting.length,scenarios:scenarios.length,steps:scenarios.reduce((sum,value)=>sum+value.steps.length,0)}));
