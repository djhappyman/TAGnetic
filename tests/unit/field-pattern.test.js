import test from 'node:test';
import assert from 'node:assert/strict';
import { patternParts, fieldSuggestions, fieldOperation, operationFields, fieldHue } from '../../app/ui/js/field-pattern.js';
import { titleCaseTagLabel } from '../../app/ui/js/tag-labels.js';
const fields = [{key:'artist',label:'Artist'}, {key:'albumartist',label:'Album Artist'}, {key:'title',label:'Title'}, {key:'custom:VENUE',label:'Venue'}];
test('chips round-trip punctuation, Unicode, custom fields and unknown native syntax exactly', () => {
  for (const pattern of ['%artist% - %title%', '%ARTIST% / %custom:VENUE%', '$if2(%albumartist%,%artist%)', '🎵  artist\n[%unknown%] 100%']) {
    assert.equal(patternParts(pattern, fields).map(part => part.text).join(''), pattern);
  }
  assert.equal(patternParts('artist', fields)[0].field, undefined, 'literal words are not committed tags');
  assert.equal(patternParts('%custom:VENUE%', fields)[0].field.key,'custom:VENUE');
});
test('autocomplete prioritizes exact/prefix matches but offers Album Artist for artist', () => {
  assert.deepEqual(fieldSuggestions(fields,'artist').map(field=>field.key), ['artist','albumartist']);
  assert.deepEqual(fieldSuggestions(fields,'%ART').map(field=>field.key), ['artist','albumartist']);
  assert.deepEqual(fieldSuggestions(fields,'nothing'), []);
  assert.equal(titleCaseTagLabel('album artist'), 'Album Artist');
  assert.equal(titleCaseTagLabel('DISCOGS_RELEASE_ID'.replaceAll('_',' ')), 'Discogs Release ID');
});
test('Build and Extract use the existing native operations and canonical destinations', () => {
  assert.deepEqual(fieldOperation('build','comment','title','%artist%'), {kind:'format',field:'comment',pattern:'%artist%'});
  const op = fieldOperation('extract','comment','title','%artist% - %title% [%custom:VENUE%]');
  assert.deepEqual(op,{kind:'extract',source:'%title%',pattern:'%artist% - %title% [%custom:VENUE%]'});
  assert.deepEqual(operationFields(op,fields),['artist','title','custom:VENUE']);
});

test('field colours stay consistent across input and preview and distinguish adjacent tags', () => {
  assert.equal(fieldHue('artist'),255);
  assert.notEqual(fieldHue('artist'),fieldHue('albumartist'));
  assert.notEqual(fieldHue('artist'),fieldHue('album'));
  assert.equal(fieldHue('custom:Venue'),fieldHue('custom:VENUE'));
});
