import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { stripTypeScriptTypes } from 'node:module';
import vm from 'node:vm';

const [root, output] = process.argv.slice(2);
const source = readFileSync(`${root}/apps/web/src/components/usage/UsageProviderChart.tsx`, 'utf8');
const tangents = source.slice(source.indexOf('function monotoneTangents('), source.indexOf('\ninterface CurveSegment'));
const scale = source.slice(source.indexOf('function niceScale('), source.indexOf('\nconst PLACEHOLDER_TICKS'));
const context = vm.createContext({});
vm.runInContext(stripTypeScriptTypes(tangents + '\n' + scale), context);
const peaks = [0, 0.0001, 0.1, 1, 2, 5, 7, 10, 20, 42, 101, 999, 12000, 1e9];
const points = [[], [[0, 2]], [[0, 0], [10, 20]], [[0, 0], [10, 0], [20, 40], [30, 0]], [[0, 2], [10, 3], [20, 40], [30, 40], [40, 0]], [[0, 0], [0, 5], [10, 1]]];
writeFileSync(output, JSON.stringify({ revision: '24e77473ecf41bc1f7142529c7911c1c1aae1d76', sourceSha256: createHash('sha256').update(source).digest('hex'), scales: peaks.map(peak => ({ peak, ...context.niceScale(peak, 4) })), curves: points.filter(points => points.length > 0).map(points => ({ points, tangents: context.monotoneTangents(points.map(([x, y]) => ({ x, y }))) })) }, null, 2) + '\n');
