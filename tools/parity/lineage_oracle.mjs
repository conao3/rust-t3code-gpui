import fs from "node:fs";
import { createHash } from "node:crypto";
import { stripTypeScriptTypes } from "node:module";
import { pairwise } from "./pairwise.mjs";
import path from "node:path";

const [sourcePath, outputPath] = process.argv.slice(2);
if (!sourcePath || !outputPath) throw new Error("Expected threadRelationships.ts and output JSON paths");
const source = fs.readFileSync(sourcePath, "utf8");
const bodyStart = source.indexOf("export type ThreadRelationshipKind");
if (bodyStart < 0) throw new Error("Missing original relationship module exports");
const body = source.slice(bodyStart);
const contractsSource = fs.readFileSync(path.resolve(path.dirname(sourcePath), "../../../contracts/src/orchestrationV2.ts"), "utf8");
const predicate = contractsSource.match(/export function isOrchestrationV2WorkActive\([\s\S]*?\n\}/)?.[0];
if (!predicate) throw new Error("Missing original active-work predicate");
const displaySource = fs.readFileSync(path.resolve(path.dirname(sourcePath), "subagentDisplay.ts"), "utf8");
const titleFunction = displaySource.match(/export function formatSubagentDisplayTitle\([\s\S]*?\n\}/)?.[0];
const runtimeSource = fs.readFileSync(path.resolve(path.dirname(sourcePath), "subagentRuntime.ts"), "utf8");
const promptFunction = runtimeSource.match(/function promptTitle\([\s\S]*?\n\}/)?.[0];
if (!titleFunction || !promptFunction) throw new Error("Missing original agent title functions");
const adapter = `const DateTime = {isDateTime: value => value instanceof Date && Number.isFinite(+value), toEpochMillis: value => +value};\n`;
const original = await import("data:text/javascript;base64," + Buffer.from(adapter + stripTypeScriptTypes(predicate + "\n" + titleFunction + "\nexport " + promptFunction + "\n" + body)).toString("base64"));
const thread = (id, parent = null, kind = null, extra = {}) => ({
  id, title: "Conversation " + id, status: "idle", activityRunStatus: null,
  latestRunId: null, forkedFrom: null, lineage: {parentThreadId: parent, relationshipToParent: kind, sideChat: false},
  createdAt: "2026-11-17T00:00:00.000Z", ...extra,
});
const projection = (selected, extra = {}) => ({thread: selected, subagents: [], contextTransfers: [], ...extra});
const cases = [];
const add = (name, threads, selected, current = selected?.thread.id ?? "current") => {
  const graph = original.deriveThreadRelationshipGraph({
    threads: threads.map(value => ({...value, createdAt: value.createdAt == null ? null : new Date(value.createdAt)})),
    projection: selected,
  });
  const rows = original.orderWebThreadLineageRows({graph,
    rows: original.immediateThreadRelationships(graph, current), currentThreadId: current,
    mergeTargetThreadId: original.resolveMergeBackTargetThreadId(selected),
  });
  cases.push({name, threads, projection: selected, current, expected: rows.map(row => {
    const node = graph.nodes.get(row.threadId);
    const shell = threads.find(value => value.id === row.threadId);
    return {threadId: row.threadId, sourceThreadId: row.edge.sourceThreadId,
      targetThreadId: row.edge.targetThreadId, kind: row.edge.kind,
      edgeStatus: row.edge.status ?? null, status: original.threadRelationshipRowStatus(graph, row),
      parent: original.isParentThreadRelationship(row.edge, current), title: node.thread?.title ?? row.threadId,
      missing: node.missing, sideChat: node.thread?.lineage.sideChat === true,
      createdAt: shell?.createdAt ?? null};
  })});
};
const root = thread("current");
add("empty", [], null);
add("parent from projection missing in shell", [], projection(thread("current", "missing", "fork")));
add("first shell is authoritative", [root, thread("child", "current", "fork", {title: "live"}), thread("child", "elsewhere", "fork", {title: "stale"})], projection(root));
add("forked run overrides lineage parent", [thread("current", "wrong", "fork", {forkedFrom: {type: "run", threadId: "right"}}), thread("right"), thread("wrong")], projection(thread("current", "wrong", "fork", {forkedFrom: {type: "run", threadId: "right"}})));
add("deduplicate related id across edge kinds", [root, thread("child", "current", "fork")], projection(root, {subagents: [{childThreadId: "child", status: "running"}], contextTransfers: [{sourceThreadId: "current", targetThreadId: "child", status: "consumed"}]}));
add("transfer ordering and self transfer", [root, thread("older", null, null, {createdAt: "2020-01-01T00:00:00.000Z"}), thread("newer"), thread("unknown", null, null, {createdAt: null})], projection(root, {contextTransfers: ["unknown", "older", "current", "newer", "older"].map(targetThreadId => ({sourceThreadId: "current", targetThreadId, status: "resolved_portable"}))}));
add("parent pinned before newest child", [thread("current", "parent", "fork"), thread("parent", null, null, {createdAt: "1960-01-01T00:00:00.000Z"}), thread("child", "current", "fork")], projection(thread("current", "parent", "fork")));
add("side chats and missing unrelated edges", [root, thread("side", "current", "fork", {lineage: {parentThreadId: "current", relationshipToParent: "fork", sideChat: true}}), thread("unrelated", "elsewhere", "subagent")], projection(root));
for (const [agentStatus, childStatus, latestRunId] of pairwise([["pending", "running", "waiting", "completed", "failed", "rolled_back", "idle", null], [null, "completed", "failed", "running"], [null, "run"]])) {
  const child = thread("child", "current", "subagent", {status: childStatus, latestRunId});
  add(`subagent ${agentStatus}/${childStatus}/${latestRunId}`, [root, child], projection(root, {subagents: [{childThreadId: "child", status: agentStatus}]}));
  add(`active child overrides ${agentStatus}/${childStatus}/${latestRunId}`, [root, {...child, activityRunStatus: "waiting"}], projection(root, {subagents: [{childThreadId: "child", status: agentStatus}]}));
}
add("omitted subagents and duplicate transfers", [root], projection(root, {subagents: [{childThreadId: null, status: "running"}, {childThreadId: "missing", status: "rolled_back"}], contextTransfers: [{sourceThreadId: "current", targetThreadId: "transfer", status: "pending"}, {sourceThreadId: "current", targetThreadId: "transfer", status: "consumed"}]}));
const titleCases = ["Subagent: /root/foo_bar", "Subagent: Subagent: /root/foo", "/root/a//b", "/root/a/", "/root//", "/root/", "/root/a//", "SUBAGENT:\uFEFF/root/foo\u3000bar", "Subagent:日本語", "/", "/root/ßeta_test", "/root/İstanbul", "/ROOT/x", "/root/a_b/c_d", "/root/___", "Subagent: ", "subagent:", "subagent::x", "Subagent:\u0085/root/a", "/root/a\u0085b", "普通の見出し"].map(input => ({input, expected: original.formatSubagentDisplayTitle(input)}));
const promptCases = ["short", "a".repeat(80), "a".repeat(81), "a".repeat(76)+"🙂".repeat(5), "日本語".repeat(30), "🙂".repeat(41)].map(input => ({input, expected: original.promptTitle(input)}));
fs.writeFileSync(outputPath, JSON.stringify({sourceRevision: "24e77473ecf41bc1f7142529c7911c1c1aae1d76", sourceSha256: createHash("sha256").update(source).digest("hex"), contractsSourceSha256: createHash("sha256").update(contractsSource).digest("hex"), displaySourceSha256: createHash("sha256").update(displaySource).digest("hex"), runtimeSourceSha256: createHash("sha256").update(runtimeSource).digest("hex"), dateAdapter: "Schema DateTimeUtc shells decoded to Date for isDateTime/toEpochMillis only", cases, titleCases, promptCases}, null, 2) + "\n");
console.log(`${cases.length} cases`);
