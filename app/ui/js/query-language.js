// Predictable, local phrase recognition. Proposals are never applied implicitly.
import {newCondition} from './query-model.js';
const aliases={artist:'artist',artists:'artist','album artist':'albumartist',genre:'genre',genres:'genre',album:'album',title:'title',year:'year',label:'label',publisher:'publisher',format:'format',bpm:'bpm',bitrate:'bitrate',duration:'duration',artwork:'cover',cover:'cover'};
const condition=(field,operator,values)=>({...newCondition(field),operator,values});
export function interpretQueryText(input){
 const text=input.trim();const nodes=[];
 // Protect literal titles/credits before recognizing words such as "missing"
 // or "or". Unfinished quotes stay text while the user is still typing.
 if(text.includes('"')){
  const quoted=text.match(/^(album artist|artists?|genres?|album|title|year|label|publisher|format|bpm|bitrate|duration)\s*(?::\s*|\s+(?:is\s+)?)"((?:[^"\\]|\\.)*)"$/i);
  if(!quoted)return {nodes:[],remaining:text,complete:false};
  const field=aliases[quoted[1].toLowerCase()],value=quoted[2].replace(/\\(["\\])/g,'$1');
  return {nodes:[{...newCondition(field,value),...(['artist','albumartist','title','album','label','publisher'].includes(field)?{operator:'contains'}:{})}],remaining:value,field,complete:true};
 }
 let remaining=text;
 const numeric=text.match(/^(year|bpm|bitrate|duration)\s+(?:is\s+)?(between\s+([\d.]+)\s+(?:and|to)\s+([\d.]+)|(below|under|less than|above|over|greater than|at least|at most)\s+([\d.]+))(?:\s*(?:kbps|hz|seconds|s))?$/i);
 if(numeric){const field=numeric[1].toLowerCase();const operator=numeric[3]?'between':({'below':'lt','under':'lt','less than':'lt','above':'gt','over':'gt','greater than':'gt','at least':'gte','at most':'lte'})[numeric[5].toLowerCase()];return {nodes:[condition(field,operator,numeric[3]?[numeric[3],numeric[4]]:[numeric[6]])],remaining:'',complete:true};}
 remaining=remaining.replace(/\b(missing|without|no)\s+(artwork|cover|artist|title|album|year|genre|label|comment)\b/gi,(_all,_op,field)=>{nodes.push(condition(aliases[field.toLowerCase()]||field.toLowerCase(),'empty',[]));return ' ';});
 remaining=remaining.replace(/\b(?:year\s+)?between\s+((?:19|20)\d{2})\s+(?:and|to)\s+((?:19|20)\d{2})\b/gi,(_all,lower,upper)=>{nodes.push(condition('year','between',[lower,upper]));return ' ';});
 remaining=remaining.replace(/\b(?:from|in|year)\s+((?:19|20)\d{2})\b/gi,(_all,year)=>{nodes.push(condition('year','equals',[year]));return ' ';});
 remaining=remaining.replace(/\s+/g,' ').replace(/^(?:and|with|where)\s+|\s+(?:and|with|where)$/gi,'').trim();
 const explicit=remaining.match(/^(album artist|artists?|genres?|album|title|year|label|publisher|format|bpm|bitrate|duration)\s*(?::\s*|\s+(?:is\s+)?)(.+)$/i);
 if(explicit){const field=aliases[explicit[1].toLowerCase()];const values=explicit[2].replace(/^"(.*)"$/,'$1').split(/\s+or\s+/i).map(v=>v.trim());const node={...newCondition(field),...(['artist','albumartist','title','album','label','publisher'].includes(field)?{operator:'contains'}:{}),values};return {nodes:[...nodes,node],remaining:values.length===1?values[0]:'',field,complete:true};}
 if(!nodes.length&&/^(?:19|20)\d{2}$/.test(remaining))return {nodes:[condition('year','equals',[remaining])],remaining,field:'year',complete:true};
 return {nodes,remaining,complete:nodes.length>0&&!remaining};
}
