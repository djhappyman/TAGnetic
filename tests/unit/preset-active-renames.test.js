import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';
const source = readFileSync(new URL('../../app/ui/js/chains.js', import.meta.url), 'utf8');
const ast = ts.createSourceFile('chains.js', source, ts.ScriptTarget.Latest, true);
const fn = ast.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === 'renameActivePreset');
function run(active, changes) {
  const context = { active: structuredClone(active), persistActive() {} };
  vm.createContext(context); vm.runInContext(fn.getText(ast), context);
  context.renameActivePreset(changes);
  return JSON.parse(JSON.stringify(context.active));
}
test('saved rename swaps preserve the original active preset identity', () => {
  assert.deepEqual(run({ generator: ['user:A'], renamer: ['user:B', 'builtin:standard values'] }, [{from:'A',to:'B'},{from:'B',to:'A'}]),
    {generator:['user:B'],renamer:['user:A','builtin:standard values']});
});
test('deleting a saved preset does not activate a new draft with its reused name', () => {
  assert.deepEqual(run({generator:['user:A','user:B']}, [{from:'A',to:'B'},{from:'B',to:null}]), {generator:['user:B']});
});
