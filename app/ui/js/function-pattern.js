// Native-backed expression presentation. The UI only decorates returned spans;
// parsing, arity, quoting and function definitions remain in Rust.
let catalogue;
let definitions=[];
const layouts=new Map();
const pending=new Map();
async function invoke(command,args) {const module=await import('./invoke.js');return module.invoke(command,args);}
export function loadTextFunctions() {
  if(!catalogue)catalogue=invoke('pattern_text_functions').then(items=>{definitions=items;return items;}).catch(error=>{catalogue=null;throw error;});
  return catalogue;
}
export async function loadFunctionLayout(pattern) {
  if(layouts.has(pattern))return layouts.get(pattern);
  if(pending.has(pattern))return pending.get(pattern);
  const request=Promise.all([loadTextFunctions(),invoke('pattern_function_layout',{pattern})]).then(([,spans])=>{
    if(layouts.size>=100)layouts.delete(layouts.keys().next().value);
    layouts.set(pattern,spans);return spans;
  }).finally(()=>pending.delete(pattern));
  pending.set(pattern,request);return request;
}
export function functionParts(value,fields,parts,enabled=true) {
  if(!enabled)return parts(value,fields);
  const result=[];let at=0;
  for(const call of layouts.get(value)||[]) {
    const fn=definitions.find(item=>item.key===call.key);if(!fn)continue;
    if(call.start>at)result.push(...parts(value.slice(at,call.start),fields));
    result.push({text:call.raw,fn,call});at=call.end;
  }
  if(at<value.length)result.push(...parts(value.slice(at),fields));
  return result;
}
export async function buildFunction(key,args,original) {
  const raw=await invoke('build_pattern_function',{key,args,original:original||null});
  await loadFunctionLayout(raw); // Commit only when the whole expression can draw atomically.
  return raw;
}
export function functionSummary(call,fields) {
  const fn=definitions.find(item=>item.key===call.key);
  const source=(call.display_args?.[0]??call.args[0]).replace(/%([^%]+)%/g,(raw,key)=>fields.find(field=>field.key.toLowerCase()===key.toLowerCase())?.label||raw);
  return `${fn?.label||call.key} · ${source.length>42?source.slice(0,39)+'…':source||'Value'}`;
}
