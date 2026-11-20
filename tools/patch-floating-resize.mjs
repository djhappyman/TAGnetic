// Floating UI 1.8.0 calls its layout update inside ResizeObserver delivery.
// Animated selects can then resize the observed surface in WebKit's same
// delivery cycle. Coalesce only resize updates into a frame; scroll/initial
// positioning stays immediate. Remove this pinned patch after an upstream fix.
import { readFile, writeFile } from 'node:fs/promises';

const root = new URL('../node_modules/@floating-ui/dom/', import.meta.url);
const metadata = JSON.parse(await readFile(new URL('package.json', root), 'utf8'));
if (metadata.version !== '1.8.0') throw new Error('Review the floating resize fix for the new Floating UI version');
const marker = '// Tagnetic: defer observed resize layout until the next frame.';
for (const name of ['floating-ui.dom.mjs', 'floating-ui.dom.esm.js', 'floating-ui.dom.umd.js']) {
  const path = new URL(`dist/${name}`, root);
  let source = await readFile(path, 'utf8');
  if (source.includes(marker)) continue;
  const resize = /^([ \t]*)update\(\);\n([ \t]*)\}\);\n\2if \(referenceEl && !animationFrame\)/m;
  const cleanup = /^([ \t]*)resizeObserver = null;\n\1if \(animationFrame\)/m;
  if (source.split('let reobserveFrame = -1;').length !== 2 || !resize.test(source) || !cleanup.test(source))
    throw new Error(`Floating UI ${name} changed; the resize fix must be reviewed`);
  source = source.replace('let reobserveFrame = -1;', `let reobserveFrame = -1;\n  ${marker}\n  let resizeUpdateFrame = -1;`);
  source = source.replace(resize, (_, indent, parent) => `${indent}if (resizeUpdateFrame < 0) resizeUpdateFrame = requestAnimationFrame(() => {
${indent}  resizeUpdateFrame = -1;
${indent}  if (resizeObserver) update();
${indent}});
${parent}});
${parent}if (referenceEl && !animationFrame)`);
  source = source.replace(cleanup, (_, indent) => `${indent}resizeObserver = null;
${indent}cancelAnimationFrame(resizeUpdateFrame);
${indent}cancelAnimationFrame(reobserveFrame);
${indent}if (animationFrame)`);
  await writeFile(path, source);
}

// Bits UI's viewport observer also writes scroll/layout during delivery. Its
// initial/keyboard alignment remains synchronous; defer only resize realignment.
const bits = new URL('../node_modules/bits-ui/', import.meta.url);
const bitsMetadata = JSON.parse(await readFile(new URL('package.json', bits), 'utf8'));
if (bitsMetadata.version !== '2.21.0') throw new Error('Review the select resize fix for the new Bits UI version');
const path = new URL('dist/bits/select/select.svelte.js', bits);
let source = await readFile(path, 'utf8');
const bitsMarker = '// Tagnetic: defer select alignment until after resize delivery.';
if (!source.includes(bitsMarker)) {
  const before = `const observer = new (this.domContext.getWindow().ResizeObserver)(() => {
                if (this.userHasScrolled || !this.root.highlightedNode)
                    return;
                this.root.scrollHighlightedNodeIntoView(this.root.highlightedNode);
            });`;
  const after = `${bitsMarker}
            const resizeWindow = this.domContext.getWindow();
            let alignmentFrame = 0;
            const observer = new resizeWindow.ResizeObserver(() => {
                if (alignmentFrame) return;
                alignmentFrame = resizeWindow.requestAnimationFrame(() => {
                    alignmentFrame = 0;
                    if (this.userHasScrolled || !this.root.highlightedNode || !this.isPositioned) return;
                    this.root.scrollHighlightedNodeIntoView(this.root.highlightedNode);
                });
            });`;
  const cleanup = '() => observer.disconnect());';
  if (source.split(before).length !== 2 || source.split(cleanup).length !== 2)
    throw new Error('Bits UI select source changed; the resize fix must be reviewed');
  source = source.replace(before, after).replace(cleanup,
    '() => { observer.disconnect(); resizeWindow.cancelAnimationFrame(alignmentFrame); });');
  await writeFile(path, source);
}
