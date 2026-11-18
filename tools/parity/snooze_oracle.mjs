import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { stripTypeScriptTypes } from "node:module";
import { pairwise, unique } from "./pairwise.mjs";

const [root, destination] = process.argv.slice(2);
if (!root || !destination) throw new Error("Expected original source root and destination");
const paths = {settled:"packages/client-runtime/src/state/threadSettled.ts",settings:"packages/contracts/src/settings.ts"};
const sources=Object.fromEntries(Object.entries(paths).map(([name,file])=>[name,fs.readFileSync(path.join(root,file),"utf8")]));
const fn=name=>{
  const value=sources.settled.match(new RegExp(`(?:export )?function ${name}\\([\\s\\S]*?\\n\\}(?=\\r?\\n|$)`))?.[0];
  if (!value) throw new Error(`Missing original function ${name}`);
  return value.startsWith("export ")?value:"export "+value;
};
const defaults=sources.settings.match(/export const DEFAULT_SNOOZE_PRESETS:[\s\S]*?\n\];/)?.[0];
const weekdays=sources.settled.match(/const WEEKDAY_INDEX = \{[\s\S]*?\n\} as const;/)?.[0];
if (!defaults || !weekdays) throw new Error("Missing original snooze constants");
const names=["hasQueuedTurnStart","pendingQuestionAt","failureEdgeAt","threadRaisedHandWhileSnoozed","canSnooze","effectiveSnoozed","threadWokeAt","snoozeTimeOfDayLabel","addSnoozeDays","pluralize","formatSnoozeDuration","capitalize","snoozePresetDaysAhead","snoozePresetLabel","snoozePresetId","resolveSnoozePreset","resolveSnoozePresets","pickSnoozePreset","snoozeWakeLabel","resolveCustomSnooze","localSnoozeDate","localSnoozeTime","isThreadWokeUnseen"];
const code=["const DateTime={formatIso:value=>new Date(value).toISOString(),makeUnsafe:value=>new Date(value),toDate:value=>new Date(value)};const QUEUED_TURN_START_GRACE_MS=120000;const DAY_MS=86400000;const HOUR_MS=3600000;",defaults,weekdays,...names.map(fn)].join("\n");
const original=await import("data:text/javascript;base64,"+Buffer.from(stripTypeScriptTypes(code)).toString("base64"));
if (new Intl.DateTimeFormat().resolvedOptions().locale!=="en-US") throw new Error("Generate this weekday oracle with the en-US default locale");
const now="2026-11-17T12:00:00.000Z";
const base={source:{pendingRuntimeRequest:null},hasPendingApprovals:false,hasPendingUserInput:false,pendingSubagentRequest:null,pendingGuestMessageCount:0,runtime:null,latestRun:null,latestUserMessageAt:null,snoozedAt:"2026-11-17T10:00:00.000Z",snoozedUntil:"2026-11-17T13:00:00.000Z",lastVisitedAt:"2026-11-17T11:00:00.000Z"};
const states=[];
const state=(extra,clock=now)=>{
  const input={...base,...extra};
  const options={now:clock};
  const wokeAt=original.threadWokeAt(input,options);
  states.push({input,now:clock,expected:{queued:original.hasQueuedTurnStart(input,options),canSnooze:original.canSnooze(input,options),raisedHand:original.threadRaisedHandWhileSnoozed(input),effective:original.effectiveSnoozed(input,options),wokeAt,wakeLabel:original.snoozeWakeLabel(input.snoozedUntil,options),unseen:original.isThreadWokeUnseen({wokeAt,settled:input.settled===true,lastVisitedAt:input.lastVisitedAt,latestRunCompletedAt:input.latestRun?.completedAt??null})}});
};
for (const [hasPendingApprovals,hasPendingUserInput,kind,pendingUserInputAt,snoozedAt] of pairwise([[false,true],[false,true],[null,"user_input","approval","auth_refresh"],[null,"invalid","2026-11-17T09:00:00.000Z","2026-11-17T10:00:00.000Z","2026-11-17T11:00:00.000Z"],[null,"invalid",base.snoozedAt]])) {
  state({hasPendingApprovals,hasPendingUserInput,pendingUserInputAt,snoozedAt,source:{pendingRuntimeRequest:kind===null?null:{kind,createdAt:"2026-11-17T11:00:00.000Z"}}});
}
for (const [status,completedAt,latestStatus] of pairwise([["running","failed","error"],[undefined,null,"invalid","2026-11-17T09:00:00.000Z","2026-11-17T11:30:00.000Z"],[null,"failed","completed"]])) state({runtime:{status,updatedAt:"2026-11-17T11:00:00.000Z"},latestRun:latestStatus===null?null:{status:latestStatus,...(completedAt===undefined?{}:{completedAt})}});
for (const [status,age,run] of pairwise([["preparing","queued","starting","running","idle"],[-120001,-120000,0,120000,120001],[null,{requestedAt:null},{startedAt:"invalid"},{startedAt:"2026-11-17T12:00:00.000Z"}]])) state({runtime:{status},latestUserMessageAt:new Date(Date.parse(now)-age).toISOString(),latestRun:run});
for (const pendingSubagentRequest of ["approval","user_input"]) state({pendingSubagentRequest});
for (const pendingGuestMessageCount of [0,1,2]) state({pendingGuestMessageCount});
for (const snoozedUntil of [null,"invalid","2026-11-17T11:00:00.000Z",now,"2026-11-17T12:00:00.001Z","2026-11-17T12:59:59.999Z","2026-11-17T13:00:00.000Z","2026-11-18T12:00:00.000Z"]) state({snoozedUntil});
for (const [lastVisitedAt,settled] of pairwise([[null,"invalid","2026-11-17T12:59:59.999Z","2026-11-17T13:00:00.000Z"],[false,true]])) state({lastVisitedAt,settled},"2026-11-17T14:00:00.000Z");
state({},"invalid");
const schedules=[];
const customs=[];
const settings=[{kind:"after",minutes:1},{kind:"after",minutes:60},{kind:"after",minutes:90},{kind:"after",minutes:180},{kind:"after",minutes:1440},{kind:"after",minutes:2880},{kind:"at",day:"today",time:"02:30"},{kind:"at",day:"today",time:"09:00"},{kind:"at",day:"today",time:"18:00"},{kind:"at",day:"tomorrow",time:"02:30"},{kind:"at",day:"tomorrow",time:"09:00"},...Object.keys({sunday:0,monday:1,tuesday:2,wednesday:3,thursday:4,friday:5,saturday:6}).map(day=>({kind:"at",day,time:"09:00"}))];
const formatTime=date=>`${String(date.getHours()).padStart(2,"0")}:${String(date.getMinutes()).padStart(2,"0")}`;
const zones=["UTC","Asia/Tokyo","America/New_York","Europe/Berlin","Pacific/Apia"];
const clocks=[now,"2026-03-07T07:30:30.123Z","2026-03-08T06:50:30.123Z","2026-03-29T00:55:00.000Z","2026-11-01T05:15:00.000Z","2026-11-01T06:15:00.000Z","2011-12-29T10:00:00.000Z"];
const configurations=[null,[],settings,[{kind:"after",minutes:60},{kind:"after",minutes:60},{kind:"at",day:"today",time:"13:00"}]];
const units=["minutes","hours","days"];
const amounts=["","0","-1","0.00000001","0.5","1","1.5","2e2","0x10","0b10","0o10","Infinity","NaN","bad","1e100"," 2 ","\uFEFF2\uFEFF","\u00852\u0085"];
const calendarInputs=[...[
  {date:"2026-03-08",time:"02:30"},{date:"2026-11-01",time:"01:30"},{date:"2030-01-01",time:"09:00"},{date:"2026-02-30",time:"12:00"},{date:"2026-12-01",time:"25:00"},{date:"bad",time:"12:00"},
].map(value=>({mode:"date",...value})),...units.map(unit=>({mode:"duration",amount:"1.5",unit}))];
const schedulePlan=[...zones.map(zone=>[zone,now,settings]),...pairwise([zones,clocks,configurations]).filter(([,clock,configured])=>clock!==now||configured!==settings)];
const customPlan=[...amounts.map((amount,index)=>["UTC",now,{mode:"duration",amount,unit:units[index%units.length]}]),...pairwise([zones,clocks,calendarInputs])];
for (const zone of zones) {
  process.env.TZ=zone;
  for (const [,clock,configured] of schedulePlan.filter(([value])=>value===zone)) {
    const date=new Date(clock);
    const options={...(configured===null?{}:{presets:configured}),formatTime};
    const expected=original.resolveSnoozePresets(date,options);
    const picks=[];
    for (const preset of [...expected,{id:"absent"}]) for (const delta of [0,90_000,7_200_000,86_400_000]) {
      const pickedAt=new Date(date.getTime()+delta);
      picks.push({id:preset.id,now:pickedAt.toISOString(),expected:original.pickSnoozePreset({shown:expected,id:preset.id,now:pickedAt,...options})??null});
    }
    schedules.push({zone,now:clock,settings:configured,expected,picks});
  }
  for (const [,clock,input] of customPlan.filter(([value])=>value===zone)) customs.push({zone,now:clock,input,expected:original.resolveCustomSnooze(input,new Date(clock))});
}
fs.writeFileSync(destination,JSON.stringify({sourceRevision:"24e77473ecf41bc1f7142529c7911c1c1aae1d76",sources:Object.fromEntries(Object.entries(paths).map(([name,file])=>[file,createHash("sha256").update(sources[name]).digest("hex")])),adapter:"Original functions execute unchanged. Effect date construction converts JS Date values; preset time labels use HH:mm, weekday labels retain the en-US system locale. Node TZ selects each actual IANA zone.",states:unique(states),schedules,customs:unique(customs)},null,2)+"\n");
console.log(JSON.stringify({states:unique(states).length,schedules:schedules.length,customs:unique(customs).length}));
