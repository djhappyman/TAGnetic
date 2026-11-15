// A Fenwick prefix index keeps measuring a newly mounted, taller staged row
// logarithmic in library size. Rebuild only when the model/baseline changes.
export class RowHeights {
  constructor() {
    this.values = new Float64Array(0);
    this.tree = new Float64Array(1);
  }
  reset(length, heightAt) {
    this.values = new Float64Array(length);
    this.tree = new Float64Array(length + 1);
    for (let index = 0; index < length; index++) {
      const height = heightAt(index);
      this.values[index] = height;
      this.tree[index + 1] += height;
      const parent = index + 1 + ((index + 1) & -(index + 1));
      if (parent <= length) this.tree[parent] += this.tree[index + 1];
    }
  }
  set(index, height) {
    const delta = height - this.values[index];
    this.values[index] = height;
    for (let node = index + 1; node < this.tree.length; node += node & -node) this.tree[node] += delta;
  }
  top(index) {
    let total = 0;
    for (let node = index; node > 0; node -= node & -node) total += this.tree[node];
    return total;
  }
  // Match the virtual renderer's half-open intervals: an exact row bottom
  // belongs to the following row; offsets beyond the list use the final row.
  indexAt(offset) {
    const length = this.values.length;
    let index = 0, total = 0;
    let bit = 1;
    while (bit * 2 <= length) bit *= 2;
    for (; bit; bit >>= 1) {
      const next = index + bit;
      if (next <= length && total + this.tree[next] <= offset) {
        index = next;
        total += this.tree[next];
      }
    }
    return Math.max(0, Math.min(index, length - 1));
  }
}
