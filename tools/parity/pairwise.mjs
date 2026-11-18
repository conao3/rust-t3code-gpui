export function pairwise(dimensions) {
  if (dimensions.length < 2) return (dimensions[0] ?? []).map(value => [value]);
  const uncovered = new Set();
  for (let a = 0; a < dimensions.length; a++) for (let b = a + 1; b < dimensions.length; b++) for (let i = 0; i < dimensions[a].length; i++) for (let j = 0; j < dimensions[b].length; j++) uncovered.add(`${a}:${i}:${b}:${j}`);
  let product = [[]];
  for (const dimension of dimensions) product = product.flatMap(prefix => dimension.map((_, index) => [...prefix, index]));
  const covers = row => {
    const pairs = [];
    for (let a = 0; a < row.length; a++) for (let b = a + 1; b < row.length; b++) {
      const key = `${a}:${row[a]}:${b}:${row[b]}`;
      if (uncovered.has(key)) pairs.push(key);
    }
    return pairs;
  };
  const rows = [];
  const take = row => {
    for (const key of covers(row)) uncovered.delete(key);
    rows.push(row);
  };
  for (let dimension = 0; dimension < dimensions.length; dimension++) for (let index = dimension ? 1 : 0; index < dimensions[dimension].length; index++) take(dimensions.map((_, other) => (other === dimension ? index : 0)));
  while (uncovered.size) {
    let best = product[0], bestPairs = covers(best);
    for (const row of product) {
      const pairs = covers(row);
      if (pairs.length > bestPairs.length) [best, bestPairs] = [row, pairs];
    }
    take(best);
  }
  return rows.map(row => row.map((index, dimension) => dimensions[dimension][index]));
}

export const unique = cases => cases.filter((value, index) => cases.findIndex(other => JSON.stringify(other) === JSON.stringify(value)) === index);
