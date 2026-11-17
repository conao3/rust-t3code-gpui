const { EventEmitter } = require('node:events');
const { readFileSync, appendFileSync } = require('node:fs');
const { join } = require('node:path');

if (!process.env.T3_PARITY_OWNER || !process.env.T3CODE_HOME || !process.env.T3_PARITY_PR_FIXTURE) {
  throw new Error('PR RPC fixtures require an owned parity server and fixture path');
}
const original = EventEmitter.prototype.emit;
EventEmitter.prototype.emit = function (event, ...args) {
  if (event !== 'message' || this._isServer !== true || typeof this.send !== 'function') return original.call(this, event, ...args);
  let request;
  try { request = JSON.parse(args[0].toString()); } catch { return original.call(this, event, ...args); }
  if (request._tag !== 'Request' || !request.tag.startsWith('pullRequests.')) return original.call(this, event, ...args);
  const fixture = JSON.parse(readFileSync(process.env.T3_PARITY_PR_FIXTURE, 'utf8'));
  appendFileSync(join(process.env.T3CODE_HOME, 'pr-rpc.jsonl'), JSON.stringify({ method: request.tag, input: request.payload }) + '\n');
  if (request.tag === 'pullRequests.subscribeRefreshes') return true;
  const response = fixture[request.tag];
  const exit = response === undefined
    ? { _tag: 'Failure', cause: [{ _tag: 'Die', defect: 'Unconfigured fixture RPC: ' + request.tag }] }
    : { _tag: 'Success', value: response };
  this.send(JSON.stringify({ _tag: 'Exit', requestId: request.id, exit }));
  return true;
};
