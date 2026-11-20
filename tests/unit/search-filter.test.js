import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { parseSearchFields, validFilterValue, filterSuggestions, createSearchDrafts, onlineSearchDrafts, searchRequestFor } from '../../app/ui/js/search-filter.js';
const catalogue = readFileSync(new URL('../../crates/core/src/search-fields.tsv', import.meta.url), 'utf8');
test('suggestions follow the exact native source catalogue', () => {
  assert.equal(parseSearchFields(catalogue,'discogs').find(f=>f.key==='label').parameter,'label');
  assert.equal(parseSearchFields(catalogue,'musicbrainz').find(f=>f.key==='release_title').parameter,'release');
  assert.equal(parseSearchFields(catalogue,'beatport').find(f=>f.key==='label').parameter,'label_name');
  for (const source of ['bandcamp','soundeo','freedb','gnudb']) assert.deepEqual(parseSearchFields(catalogue,source),[]);
  assert.deepEqual(filterSuggestions(parseSearchFields(catalogue,'beatport'),'album'),[]);
  assert.equal(filterSuggestions(parseSearchFields(catalogue,'discogs'),'album')[0].key,'release_title');
});
test('one filter replaces the previous value and drafts remain source-specific', () => {
  const drafts=createSearchDrafts();
  drafts.set('discogs','Bonkers',{key:'label',value:'React'});
  drafts.set('musicbrainz','Bonkers',{key:'artist',value:'Hixxy'});
  drafts.set('discogs','Bonkers',{key:'artist',value:'Sharkey'});
  assert.deepEqual(drafts.get('discogs'),{text:'Bonkers',filter:{key:'artist',value:'Sharkey'}});
  assert.equal(drafts.get('musicbrainz').filter.value,'Hixxy');
  assert.equal(drafts.get('bandcamp').filter,null);
});
test('request snapshots keep typed text separate from the filter, with no hidden second medium', () => {
  onlineSearchDrafts.set('discogs','Bonkers',{key:'label',value:'React'});
  const request=searchRequestFor('discogs',' Bonkers ','CD');
  assert.deepEqual(request,{album:'Bonkers',format:null,filters:{label:'React'}});
  onlineSearchDrafts.set('discogs','new text',{key:'artist',value:'Hixxy'});
  assert.deepEqual(request.filters,{label:'React'});
  assert.deepEqual(searchRequestFor('bandcamp','https://example.bandcamp.com/album/test'),{album:'https://example.bandcamp.com/album/test',format:null,filters:{}});
});
test('empty, oversized, enum and numeric values are validated before confirmation', () => {
  const fields=parseSearchFields(catalogue,'musicbrainz');
  const label=fields.find(f=>f.key==='label'), medium=fields.find(f=>f.key==='medium'), tracks=fields.find(f=>f.key==='tracks');
  assert.equal(validFilterValue(label,'   '),false);
  assert.equal(validFilterValue(label,'é'.repeat(513)),false);
  assert.equal(validFilterValue(label,'React (2)'),true);
  assert.equal(validFilterValue(medium,'Album'),false);
  assert.equal(validFilterValue(medium,'CD'),true);
  for(const text of ['0','-1','2 OR 3','4294967296']) assert.equal(validFilterValue(tracks,text),false);
  assert.equal(validFilterValue(tracks,'32'),true);
});
