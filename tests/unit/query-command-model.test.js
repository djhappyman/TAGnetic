import test from 'node:test';
import assert from 'node:assert/strict';
import {emptyRoot,newCondition,addSuggestion,normalizeRoot,normalizeQuery,removeNode,queryLabel} from '../../app/ui/js/query-model.js';
import {mockMatches,mockSuggestions} from '../../app/ui/js/query-mock.js';
import {createLibraryQueryClient} from '../../app/ui/js/library-query-client.js';
import {normalizeSession} from '../../app/ui/js/session-resume.js';
const tracks=[
 {path:'/music/a.mp3',format:'Mp3',has_cover:false,tags:{genre:'Hardcore',artist:'Alpha',year:'1995',title:'Original Mix'}},
 {path:'/music/b.flac',format:'Flac',has_cover:true,tags:{genre:'Jungle',artist:'Beta',year:'1996',title:'Original Mix'}},
 {path:'/music/c.mp3',format:'Mp3',has_cover:true,tags:{genre:'House',artist:'Alpha',year:'2005',title:'Radio Edit'}},
];
const q=root=>normalizeQuery({text:'',root});
test('same-field suggestions become alternatives while text narrows the selected genres',()=>{
 let root=addSuggestion(emptyRoot(),{field:'genre',value:'Hardcore'});
 root=addSuggestion(root,{field:'genre',value:'Jungle'});
 assert.equal(root.children.length,1);
 assert.deepEqual(mockMatches(tracks,q(root)),[0,1]);
 assert.deepEqual(mockMatches(tracks,{...q(root),text:'Beta'}),[1]);
 assert.equal(queryLabel(root.children[0]),'Genre: Hardcore or Jungle');
});
test('nested alternatives, numeric bounds, exclusions and missing values compose',()=>{
 const genre={...newCondition('genre','Hardcore'),values:['Hardcore','Jungle']};
 const years={...newCondition('year','1995'),operator:'between',values:['1995','1996']};
 const alternatives={kind:'group',id:'either',mode:'any',children:[{...newCondition('cover',''),operator:'empty'},newCondition('artist','Beta')]};
 const root={...emptyRoot(),children:[genre,years,alternatives]};
 assert.deepEqual(mockMatches(tracks,q(root)),[0,1]);
 assert.deepEqual(mockMatches(tracks,q({...root,children:[...root.children,{...newCondition('format','Flac'),negate:true}]})),[0]);
 assert.deepEqual(mockMatches(tracks,q(removeNode(root,genre.id))),[0,1]);
});
test('legacy presets migrate and malformed nested data never silently broadens a search',()=>{
 const query=normalizeQuery({text:'',conditions:[{id:'legacy',field:'artist',operator:'contains',value:'Alpha'}],mode:'all'});
 assert.deepEqual(mockMatches(tracks,query),[0,2]);
 for(const root of [{...emptyRoot(),mode:'invented'},{...emptyRoot(),children:[{...newCondition(),field:'unknown'}]},{...emptyRoot(),children:[{...newCondition('year'),operator:'between',values:['2000','1990']}]}]) assert.throws(()=>normalizeRoot(root));
 const node=newCondition('genre','Hardcore');assert.throws(()=>normalizeRoot({...emptyRoot(),children:[node,node]}));
});
test('saved session keeps library root distinct from the query root and omits pending edits',()=>{
 const queryRoot={...emptyRoot(),children:[newCondition('genre','Hardcore')]};
 const state=normalizeSession({version:1,root:'/music',queryRoot,commandMode:'duplicates',duplicateKind:'likely',edits:{artist:'pending'}});
 assert.equal(state.root,'/music');assert.deepEqual(state.queryRoot,queryRoot);assert.equal(state.edits,undefined);
});
test('native client rejects superseded results and reuses each snapshot manifest',async()=>{
 const pending=[];const client=createLibraryQueryClient((_cmd,args)=>new Promise(resolve=>pending.push({args,resolve})));
 const first=client.run(q(emptyRoot())),second=client.run(q(emptyRoot()));
 pending[1].resolve({requestId:2,snapshotId:7,paths:['a','b'],indices:[1],groups:[]});
 assert.deepEqual((await second).matchedPaths,['b']);
 pending[0].resolve({requestId:1,snapshotId:6,paths:['obsolete'],indices:[0]});assert.equal(await first,null);
 const third=client.run(q(emptyRoot()));assert.equal(pending[2].args.request.snapshotId,7);
 pending[2].resolve({snapshotId:7,indices:[0],groups:[{indices:[0,1]}]});assert.deepEqual((await third).groups[0].paths,['a','b']);
});

test('artist member chips include collaborations without substring or punctuation damage',()=>{
 const rows=[
  {path:'a',tags:{artist:'Hixxy & Sharkey'}}, {path:'b',tags:{artist:'hixxy / Dougal'}},
  {path:'c',tags:{artist:'Hixxy, Seduction'}}, {path:'d',tags:{artist:'Hixxy2'}},
  {path:'e',tags:{artist:'AC/DC'}}, {path:'f',tags:{artist:'Earth, Wind & Fire'}},
 ];
 const root={...emptyRoot(),children:[newCondition('artist','HIXXY')]};
 assert.deepEqual(mockMatches(rows,q(root)),[0,1,2]);
 assert.deepEqual(mockMatches(rows,q({...root,children:[newCondition('artist','AC/DC')]})),[4]);
 assert.deepEqual(mockMatches(rows,q({...root,children:[newCondition('artist','Wind')]})),[]);
 assert.deepEqual(mockMatches(rows,q({...root,children:[{...newCondition('artist','Hixxy'),operator:'equals'}]})),[]);
 const suggestions=mockSuggestions(rows,{query:q(emptyRoot()),text:'hIxXy',field:'artist'});
 assert.equal(suggestions.find(s=>s.value.toLowerCase()==='hixxy').count,3);
});
