// Browser-fixture reference only. Shipping Tauri searches run in Rust.
import { QUERY_FIELDS, normalizeQuery, scopedText } from './query-model.js';
export function mockMembers(value,field){const full=value.trim().replace(/\s+/g,' ');if(field==='genre')return [...new Set([full,...full.split(/[;\n\0]+/).map(v=>v.trim()).filter(Boolean)])];const strong=full.split(/\s*(?:[;\n\0]+|\b(?:feat(?:uring)?|ft|vs)\.?\s+)\s*/i);return [...new Set([full,...strong.flatMap(part=>part.includes(',')&&part.includes('&')?[part]:part.split(/\s+[/&]\s+|,\s*/)).map(v=>v.trim()).filter(Boolean)])];}
const searchCache=new WeakMap();
function searchValue(track,folded){let cached=searchCache.get(track);if(!cached||cached.tags!==track.tags||cached.path!==track.path){const text=[mockField(track,'file'),...Object.values(track.tags)].join(' ');cached={tags:track.tags,path:track.path,text,lower:text.toLowerCase()};searchCache.set(track,cached);}return folded?cached.lower:cached.text;}
export function mockField(track,field) {
  if(field==='cover')return track.unreadable||typeof track.has_cover!=='boolean'?'unknown':track.has_cover?'embedded':'missing';
  if(field==='file')return track.path.split(/[\\/]/).pop();
  if(field==='path')return track.path;
  if(field==='format')return track.format||'';
  if(field==='duration'||field==='length')return String(track.duration_secs??'');
  if(field==='bitrate')return String(track.bitrate_kbps??'');
  if(field==='sample_rate')return String(track.sample_rate_hz??'');
  if(field==='album_artist'||field==='albumartist')return track.tags.albumartist||'';
  if(field==='position')return track.tags.track||'';
  return track.tags[field]||'';
}
export function mockMatches(tracks,input) {
  const query=normalizeQuery(input),fold=v=>query.caseSensitive?String(v):String(v).toLowerCase();
  function condition(node,t) {
    if(node.kind==='group')return node.mode==='any'?node.children.some(n=>condition(n,t)):node.children.every(n=>condition(n,t));
    const value=mockField(t,node.field),hay=fold(value),number=value.trim()?Number(value):NaN;
    let match;
    switch(node.operator){
      case 'member':match=node.values.some(v=>mockMembers(value,node.field).some(m=>fold(m)===fold(v)));break;
      case 'empty':match=!value.trim();break;
      case 'present':match=!!value.trim();break;
      case 'gt':match=number>Number(node.values[0]);break;
      case 'gte':match=number>=Number(node.values[0]);break;
      case 'lt':match=number<Number(node.values[0]);break;
      case 'lte':match=number<=Number(node.values[0]);break;
      case 'between':match=number>=Number(node.values[0])&&number<=Number(node.values[1]);break;
      default:match=node.values.some(v=>node.operator==='equals'?hay===fold(v):node.operator==='starts'?hay.startsWith(fold(v)):hay.includes(fold(v)));
    }
    return node.negate?!match:match;
  }
  let text=query.text,field=null;
  const prefix=scopedText(text);
  if(prefix){field=prefix.field;text=prefix.text;}
  let coverExact=null;
  if(field==='cover'&&!query.regex){const aliases={embedded:'embedded',missing:'missing',unknown:'unknown','no cover':'missing','no embedded artwork':'missing','embedded artwork':'embedded','artwork status unavailable':'unknown'};coverExact=aliases[fold(text)]||null;if(coverExact)text=coverExact;}
  let regex=null;
  if(query.regex&&text){if(/\(\?[=!<]|\\[1-9]/.test(text))throw Error('Lookaround and backreferences are not supported by the bounded regex engine');regex=new RegExp(text,query.caseSensitive?'':'i');}
  const indices=[];
  tracks.forEach((t,i)=>{
    if(!condition(query.root,t))return;
    if(!text){indices.push(i);return;}
    const hay=field?mockField(t,field):searchValue(t,!query.caseSensitive&&!regex);
    const label=field==='cover'?(input.coverLabels?.[hay]||({embedded:'Embedded artwork',missing:'No embedded artwork',unknown:'Artwork status unavailable'})[hay]):'';
    if(coverExact?hay===coverExact:!text||(regex?regex.test(hay)||(label&&regex.test(label)):fold(hay).includes(fold(text))||(label&&fold(label).includes(fold(text)))))indices.push(i);
  });
  return indices;
}
export function mockSuggestions(tracks,request) {
  const query={...request.query,text:''},needle=request.text.toLowerCase(),out=[];
  for(const field of QUERY_FIELDS.filter(f=>!request.field||f.key===request.field)){
    if(request.strategy==='fields'&&!request.field&&!['genre','artist','albumartist','album','title','year','label','format'].includes(field.key))continue;
    const context={...query,root:query.root.mode==='all'?{...query.root,children:query.root.children.filter(n=>!(n.kind==='condition'&&n.field===field.key&&['equals','member',...(request.strategy==='fields'?['contains']:[])].includes(n.operator)&&!n.negate))}:query.root};
    const indices=mockMatches(tracks,context);
    if(request.strategy==='fields'){
      const categorical=['genre','year','format','cover'].includes(field.key);
      const exact=categorical?indices.flatMap(i=>field.key==='genre'?mockMembers(mockField(tracks[i],field.key),field.key):[mockField(tracks[i],field.key)]).find(value=>value.toLowerCase()===needle):null;
      const operator=exact?(field.key==='genre'?'member':'equals'):'contains';
      const fold=value=>query.caseSensitive?value:value.toLowerCase(),value=fold(exact||request.text);
      const count=indices.filter(i=>{const raw=mockField(tracks[i],field.key);return exact?(operator==='member'?mockMembers(raw,field.key).some(v=>fold(v)===value):fold(raw)===value):fold(raw).includes(value);}).length;
      if(count)out.push({field:field.key,value:exact||request.text,count,operator});
      continue;
    }
    const values=new Map();
    for(const i of indices){const seen=new Set();const raw=mockField(tracks[i],field.key);for(const value of ['artist','albumartist','genre'].includes(field.key)?mockMembers(raw,field.key):[raw]){const key=value.toLowerCase();if(value&&key.includes(needle)&&!seen.has(key)){seen.add(key);const entry=values.get(key)||{value,count:0};entry.count++;values.set(key,entry);}}}
    for(const {value,count} of values.values())out.push({field:field.key,value,count,...(['artist','albumartist','genre'].includes(field.key)?{operator:'member'}:{})});
  }
  return out.sort((a,b)=>b.count-a.count||a.field.localeCompare(b.field)||a.value.localeCompare(b.value)).slice(0,request.limit||12);
}
