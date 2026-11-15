// Small, serializable command-bar state. Library data belongs to the native index.
export const QUERY_FIELDS = [
  ['artist','Artist'], ['album','Album'], ['title','Title'], ['genre','Genre'],
  ['year','Year','number'], ['label','Label'], ['comment','Comment'],
  ['file','Filename'], ['path','Folder / path'], ['format','Format'],
  ['cover','Artwork'], ['duration','Duration (seconds)','number'],
  ['bitrate','Bitrate (kbps)','number'], ['sample_rate','Sample rate (Hz)','number'],
  ['track','Track'], ['disc','Disc'], ['position','Position'],
  ['bpm','BPM','number'], ['key','Key'], ['isrc','ISRC'],
  ['albumartist','Album artist'], ['composer','Composer'], ['publisher','Publisher'],
  ['tracktotal','Track total','number'], ['disctotal','Disc total','number'],
  ['catalognumber','Catalogue number'], ['url','URL'], ['media','Media'],
  ['length','Length (seconds)','number'], ['tagtypes','Tag types'],
].map(([key,label,type='text'])=>({key,label,type}));
export function registerQueryFields(fields) {
  for(const field of fields) if(typeof field.key==='string'&&field.key.length<=128&&!QUERY_FIELDS.some(f=>f.key===field.key)) QUERY_FIELDS.push({type:'text',...field});
}
export const QUERY_OPERATORS = [
  ['member','includes a value'], ['equals','is exactly'], ['contains','contains'], ['starts','starts with'],
  ['empty','is missing'], ['present','is present'],
  ['gt','is greater than'], ['gte','is at least'], ['lt','is less than'],
  ['lte','is at most'], ['between','is between'],
].map(([key,label])=>({key,label}));
let sequence=0;
export const queryId=()=>`query-${Date.now()}-${++sequence}`;
export const emptyRoot=()=>({kind:'group',id:'root',mode:'all',children:[]});
export const defaultOperator=field=>['artist','albumartist','album_artist','genre'].includes(field)?'member':'equals';
export const newCondition=(field='genre',value='')=>({kind:'condition',id:queryId(),field,operator:defaultOperator(field),values:[value],negate:false});
export const fieldLabel=key=>QUERY_FIELDS.find(f=>f.key===key)?.label||key;
const queryTones={artist:'blue',albumartist:'purple',album_artist:'purple',genre:'green',title:'amber',album:'cyan',year:'orange',cover:'pink'};
export const queryTone=field=>queryTones[field]||'purple';
export function compactQueryLabel(node) {
  if(node.kind==='group')return `${node.mode==='any'?'Any':'All'}: ${node.children.map(compactQueryLabel).join(' / ')}`;
  if(!['equals','member','contains'].includes(node.operator))return queryLabel(node);
  return `${node.negate?'Not ':''}${node.values.join(' or ')}`;
}
export function scopedText(text) {
  const field=[...QUERY_FIELDS].sort((a,b)=>b.key.length-a.key.length).find(f=>text.toLowerCase().startsWith(`${f.key.toLowerCase()}:`));
  return field?{field:field.key,text:text.slice(field.key.length+1)}:null;
}
/** @param {any} value */
export function normalizeRoot(value=emptyRoot()) {
  let count=0;
  const ids=new Set();
  function visit(node,depth) {
    if(++count>128||depth>8) throw new Error('Use at most 128 conditions and eight group levels.');
    if(!node||typeof node.id!=='string'||!node.id||node.id.length>100||ids.has(node.id)) throw new Error('Invalid filter condition identifier.');
    ids.add(node.id);
    if(node.kind==='group') {
      if(!['all','any'].includes(node.mode)||!Array.isArray(node.children)) throw new Error('Choose match all or match any for each group.');
      if(depth&&node.children.length===0) throw new Error('Add a condition to the empty group.');
      return {kind:'group',id:node.id,mode:node.mode,children:node.children.map(child=>visit(child,depth+1))};
    }
    if(node.field==='cover'&&['empty','present'].includes(node.operator)) node={...node,operator:'equals',values:[node.operator==='empty'?'missing':'embedded']};
    if(node.kind!=='condition'||!(QUERY_FIELDS.some(f=>f.key===node.field)||/^custom:[^\0]{1,120}$/.test(node.field))||!QUERY_OPERATORS.some(o=>o.key===node.operator)||!Array.isArray(node.values)||node.values.length>32) throw new Error('Choose a supported field and matching rule.');
    const rawValues=node.values.map(v=>{
      if(typeof v!=='string'||v.length>512) throw new Error('Filter values must be at most 512 characters.');
      return v.trim();
    });
    const values=node.operator==='between'?rawValues:[...new Set(rawValues)];
    if(!['empty','present'].includes(node.operator)&&(!values.length||values.some(v=>!v))) throw new Error('Complete each filter value.');
    if(['gt','gte','lt','lte','between'].includes(node.operator)) {
      if(QUERY_FIELDS.find(f=>f.key===node.field)?.type!=='number'||values.some(v=>!Number.isFinite(Number(v)))||values.length!==(node.operator==='between'?2:1)) throw new Error('Enter valid numeric bounds.');
      if(node.operator==='between'&&Number(values[0])>Number(values[1])) throw new Error('The lower bound must come first.');
    }
    return {kind:'condition',id:node.id,field:node.field,operator:node.operator,values,negate:node.negate===true};
  }
  if(value?.kind!=='group') throw new Error('A filter needs a root group.');
  return visit(value,0);
}
export function legacyRoot(conditions=[],mode='all') {
  return normalizeRoot({kind:'group',id:'root',mode,children:conditions.map(c=>c.kind?c:{kind:'condition',id:c.id,field:c.field,operator:c.operator,values:[c.value],negate:false})});
}
export function queryLabel(node) {
  if(node.field==='cover'&&['empty','present'].includes(node.operator)) return `Artwork: ${node.operator==='empty'?'missing':'embedded'}`;
  if(node.kind==='group') return `${node.mode==='any'?'Any':'All'} of: ${node.children.map(queryLabel).join(node.mode==='any'?' / ':'; ')}`;
  const op=QUERY_OPERATORS.find(o=>o.key===node.operator)?.label||node.operator;
  const label=fieldLabel(node.field);
  const values=node.values.join(node.operator==='between'?' and ':' or ');
  return `${node.negate?'Not ':''}${label}${['equals','member'].includes(node.operator)?':':` ${op}${['empty','present'].includes(node.operator)?'':':'}`}${['empty','present'].includes(node.operator)?'':` ${values}`}`;
}
export function replaceNode(root,id,next) {
  if(root.id===id) return next;
  return root.kind==='group'?{...root,children:root.children.map(n=>replaceNode(n,id,next))}:root;
}
export function removeNode(root,id) {
  return {...root,children:root.children.filter(n=>n.id!==id).map(n=>n.kind==='group'?removeNode(n,id):n).filter(n=>n.kind!=='group'||n.children.length)};
}
export function addSuggestion(root,suggestion) {
  const current=root.children.find(n=>n.kind==='condition'&&n.field===suggestion.field&&n.operator===(suggestion.operator||defaultOperator(suggestion.field))&&!n.negate);
  if(current&&root.mode==='all') return replaceNode(root,current.id,{...current,values:[...new Set([...current.values,suggestion.value])]});
  return {...root,children:[...root.children,{...newCondition(suggestion.field,suggestion.value),operator:suggestion.operator||defaultOperator(suggestion.field)}]};
}
export function appendQueryNodes(root,nodes) {
  let next=root;
  for(const node of nodes){
    const old=next.mode==='all'&&node.kind==='condition'&&!node.negate&&['equals','member'].includes(node.operator)?next.children.find(n=>n.kind==='condition'&&n.field===node.field&&n.operator===node.operator&&!n.negate):null;
    next=old?replaceNode(next,old.id,{...old,values:[...new Set([...old.values,...node.values])]}):{...next,children:[...next.children,node]};
  }
  return next;
}
export function normalizeQuery(value) {
  const root=value.root?normalizeRoot(value.root):legacyRoot(value.conditions||[],value.mode==='any'?'any':'all');
  if(typeof value.text!=='string'||value.text.length>2048) throw new Error('Filter text must be at most 2048 characters.');
  return {version:2,text:value.text,regex:!!value.regex,caseSensitive:!!value.caseSensitive,root,
    mode:value.commandMode==='duplicates'||value.mode==='duplicates'?'duplicates':'search',duplicateKind:['exact','same_audio','likely'].includes(value.duplicateKind)?value.duplicateKind:'exact'};
}
