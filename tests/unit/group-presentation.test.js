import test from 'node:test';
import assert from 'node:assert/strict';
import {COMMON_GROUPS, groupIcon, folderDisplayLabels} from '../../app/ui/js/group-presentation.js';

test('common grouping choices have a shared, stable icon mapping', () => {
  assert.deepEqual(COMMON_GROUPS, ['folder','album','albumartist','artist','disc','year','genre','media']);
  assert.equal(new Set(COMMON_GROUPS.map(groupIcon)).size, 8);
  assert.equal(groupIcon('publisher'), 'tag');
});
test('folder labels omit only shared leading directories and preserve the keys', () => {
  const paths = ['/Music/Music Testing/Bonkers/Pack 2/CD 1','/Music/Music Testing/Bonkers/Pack 2/CD 2','/Music/Music Testing/HTID/Pack 1/CD 1'];
  const original = [...paths];
  const labels = folderDisplayLabels(paths);
  assert.deepEqual([...labels.values()], ['Bonkers/Pack 2/CD 1','Bonkers/Pack 2/CD 2','HTID/Pack 1/CD 1']);
  assert.deepEqual([...labels.keys()], original);
  assert.deepEqual(paths, original);
});
test('single folders, duplicates and ancestor folders always retain a meaningful leaf', () => {
  assert.deepEqual([...folderDisplayLabels(['/music/Pack','/music/Pack']).values()], ['Pack']);
  assert.deepEqual([...folderDisplayLabels(['/music/Pack','/music/Pack/CD 1']).values()], ['Pack','Pack/CD 1']);
  assert.deepEqual([...folderDisplayLabels(['/music/Pack','/music/Packs']).values()], ['Pack','Packs']);
  assert.deepEqual([...folderDisplayLabels([])], []);
  assert.equal(folderDisplayLabels(['/']).get('/'), '/');
});
test('Windows separators and UNC paths trim at complete component boundaries', () => {
  const keys = ['C:\\Music\\Packs\\Bonkers\\CD1', 'C:/Music/Packs/HTID/CD2'];
  assert.deepEqual([...folderDisplayLabels(keys).values()], ['Bonkers/CD1','HTID/CD2']);
  assert.deepEqual([...folderDisplayLabels(['//server/share/Pack/CD1','//server/share/Pack/CD2']).values()], ['CD1','CD2']);
  assert.deepEqual([...folderDisplayLabels(['C:/Music/Pack','D:/Music/Pack']).values()], ['C:/Music/Pack','D:/Music/Pack']);
});
test('a large folder set is processed once without losing distinct branches', () => {
  const paths = Array.from({length:100000}, (_,i) => `/Music/Packs/Pack ${i}/CD 1`);
  const labels = folderDisplayLabels(paths);
  assert.equal(labels.size, 100000);
  assert.equal(labels.get(paths[0]), 'Pack 0/CD 1');
  assert.equal(labels.get(paths.at(-1)), 'Pack 99999/CD 1');
});
