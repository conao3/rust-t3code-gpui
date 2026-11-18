export function boundaries(dimensions, evaluate, outcome = value => value.expected) {
  let rows = [[]];
  for (const dimension of dimensions) rows = rows.flatMap(prefix => dimension.map((_, index) => [...prefix, index]));
  const cases = rows.map(row => evaluate(row.map((index, dimension) => dimensions[dimension][index])));
  const outcomes = cases.map(value => JSON.stringify(outcome(value)));
  const kept = new Set();
  const seenOutcomes = new Set();
  outcomes.forEach((value, index) => {
    if (!seenOutcomes.has(value)) kept.add(index);
    seenOutcomes.add(value);
  });
  const flips = new Map();
  dimensions.forEach((_, dimension) => {
    const groups = new Map();
    rows.forEach((row, index) => {
      const key = row.filter((_, other) => other !== dimension).join(",");
      groups.set(key, [...(groups.get(key) ?? []), index]);
    });
    for (const members of groups.values()) for (let a = 0; a < members.length; a++) for (let b = a + 1; b < members.length; b++) {
      const [left, right] = [members[a], members[b]];
      if (outcomes[left] === outcomes[right]) continue;
      const flip = `${dimension}:${rows[left][dimension]}:${rows[right][dimension]}`;
      flips.set(flip, [...(flips.get(flip) ?? []), [left, right]]);
    }
  });
  for (const pairs of flips.values()) {
    if (pairs.some(([left, right]) => kept.has(left) && kept.has(right))) continue;
    const [left, right] = pairs.find(([left, right]) => kept.has(left) || kept.has(right)) ?? pairs[0];
    kept.add(left);
    kept.add(right);
  }
  dimensions.forEach((dimension, position) => dimension.forEach((_, value) => {
    if (![...kept].some(index => rows[index][position] === value)) kept.add(rows.findIndex(row => row[position] === value));
  }));
  return cases.filter((_, index) => kept.has(index));
}

export const unique = cases => cases.filter((value, index) => cases.findIndex(other => JSON.stringify(other) === JSON.stringify(value)) === index);
