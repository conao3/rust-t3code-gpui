const { EventEmitter } = require('node:events');
const { readFileSync, appendFileSync } = require('node:fs');
const { join } = require('node:path');

if (!process.env.T3_PARITY_OWNER || !process.env.T3CODE_HOME || !process.env.T3_PARITY_PR_FIXTURE) {
  throw new Error('PR RPC fixtures require an owned parity server and fixture path');
}
const original = EventEmitter.prototype.emit;
EventEmitter.prototype.emit = function (event, ...args) {
  if (event === 'request' && ['POST', 'OPTIONS'].includes(args[0]?.method) && args[0]?.url?.split('?')[0] === '/api/pull-requests/diff') {
    const headers = {
      'Content-Type': 'application/json',
      'Access-Control-Allow-Origin': args[0].headers.origin ?? '*',
      'Access-Control-Allow-Credentials': 'true',
      'Access-Control-Allow-Methods': 'POST, OPTIONS',
      'Access-Control-Allow-Headers': args[0].headers['access-control-request-headers'] ?? 'content-type, authorization',
      'Vary': 'Origin',
    };
    if (args[0].method === 'OPTIONS') {
      args[1].writeHead(204, headers);
      args[1].end();
      return true;
    }
    const fixture = JSON.parse(readFileSync(process.env.T3_PARITY_PR_FIXTURE, 'utf8'));
    const response = fixture['http:/api/pull-requests/diff'];
    const body = [];
    args[0].on('data', chunk => body.push(chunk));
    args[0].on('end', () => {
      appendFileSync(join(process.env.T3CODE_HOME, 'pr-rpc.jsonl'), JSON.stringify({ method: 'http:/api/pull-requests/diff', input: JSON.parse(Buffer.concat(body).toString()) }) + '\n');
      args[1].writeHead(response === undefined ? 500 : 200, headers);
      args[1].end(JSON.stringify(response ?? { error: 'Unconfigured fixture HTTP: /api/pull-requests/diff' }));
    });
    return true;
  }
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
