import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { stripTypeScriptTypes } from 'node:module';
import vm from 'node:vm';

const [root, output] = process.argv.slice(2);
const source = readFileSync(`${root}/packages/shared/src/usageFormat.ts`, 'utf8');
const code = stripTypeScriptTypes(source.replace(/import[\s\S]*?;\n/g, '')).replace(/export /g, '');
const context = vm.createContext({ UsageDay: { make: value => value } });
vm.runInContext(code, context);
const values = [0, 0.005, 1.005, -1.005, 999, 1000, 1200, 1234, 10450, 12250, 12500, 999999, 1e6, 1e9, 1e12, -1250];
const rows = values.map(value => ({ value, tokens: context.formatTokens(value), usd: context.formatUsd(value), count: context.formatCount(value) }));
writeFileSync(output, JSON.stringify({ revision: '24e77473ecf41bc1f7142529c7911c1aae1d76', sourceSha256: createHash('sha256').update(source).digest('hex'), rows }, null, 2) + '\n');
