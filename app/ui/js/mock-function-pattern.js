// Presentation metadata for the existing native text functions. Logic stays raw.
export const TEXT_FUNCTIONS = [
  {key:'upper',label:'Uppercase',description:'Make text uppercase.',args:['Value']},
  {key:'lower',label:'Lowercase',description:'Make text lowercase.',args:['Value']},
  {key:'trim',label:'Trim',description:'Remove outer spaces.',args:['Value']},
  {key:'strpos',label:'Find',description:'Find a text position.',args:['Value','Find']},
  {key:'replace',label:'Replace',description:'Replace matching text.',args:['Value','Find','Replace with']},
];
export function textFunction(key) { return TEXT_FUNCTIONS.find(fn=>fn.key===key.toLowerCase()); }
// Scan only complete native calls, retaining exact argument spelling. Rust still
// owns validity and evaluation; this scanner never executes an expression.
export function readFunction(value, start=0) {
  const match=value.slice(start).match(/^\$([a-z][a-z0-9]*)\(/i);
  if(!match)return null;
  const args=[];let depth=1,section=0,quoted=false,begin=start+match[0].length;
  for(let at=begin;at<value.length;at++) {
    const ch=value[at];
    if(ch==="'"){quoted=!quoted;continue;}
    if(quoted)continue;
    if(ch==='[')section++;else if(ch===']')section--;
    if(ch==='(')depth++;
    if(ch===')') {
      depth--;if(!depth){args.push(value.slice(begin,at));return {key:match[1].toLowerCase(),args,end:at+1,raw:value.slice(start,at+1)};}
    }
    if(ch===','&&depth===1&&section===0){args.push(value.slice(begin,at));begin=at+1;}
  }
  return null;
}
export function functionParts(value, fields, parts, enabled=true) {
  if(!enabled)return parts(value,fields);
  const result=[];let start=0,quoted=false;
  for(let at=0;at<value.length;at++) {
    if(value[at]==="'"){quoted=!quoted;continue;}
    if(quoted||value[at]!=='$')continue;
    const call=readFunction(value,at);if(!call)continue;
    const fn=textFunction(call.key);
    if(fn&&fn.args.length===call.args.length){
      if(at>start)result.push(...parts(value.slice(start,at),fields));
      result.push({text:call.raw,fn,call});start=call.end;
    }
    at=call.end-1;
  }
  if(start<value.length)result.push(...parts(value.slice(start),fields));
  return result;
}
// Preserve calls and placeholders. Quote reserved literal
// characters introduced in argument fields, including commas and apostrophes.
export function encodeArgument(value) {
  let out='';
  for(let at=0;at<value.length;) {
    const call=value[at]==='$'&&readFunction(value,at);
    if(call){out+=call.raw;at=call.end;continue;}
    const tag=value.slice(at).match(/^%[^%]+%/);
    if(tag){out+=tag[0];at+=tag[0].length;continue;}
    if(value[at]==="'"){out+="''";at++;continue;}
    const ch=value[at++];out+=/[(),\[\]%$]/.test(ch)?`'${ch}'`:ch;
  }
  return out;
}
export function argumentDisplay(raw) {
  let out='';
  for(let at=0;at<raw.length;) {
    const call=raw[at]==='$'&&readFunction(raw,at);
    if(call){out+=call.raw;at=call.end;continue;}
    if(raw[at]==="'") {
      const close=raw.indexOf("'",at+1);
      if(close>=0){const span=raw.slice(at+1,close);out+=/[%$]/.test(span)?raw.slice(at,close+1):span||"'";at=close+1;continue;}
    }
    out+=raw[at++];
  }
  return out;
}
export function buildFunction(key,args,preserved=[]) {
  const fn=textFunction(key);
  if(!fn||args.length!==fn.args.length)throw new Error('Unsupported function arguments');
  return `$${key}(${args.map((value,index)=>preserved[index]?.value===value?preserved[index].raw:encodeArgument(value)).join(',')})`;
}
export function functionSummary(call,fields) {
  const fn=textFunction(call.key);if(!fn)return call.raw;
  const value=call.args[0].replace(/%([^%]+)%/g,(raw,key)=>fields.find(field=>field.key.toLowerCase()===key.toLowerCase())?.label||raw);
  const nested=readFunction(value);
  const source=nested?.end===value.length?(textFunction(nested.key)?.label||value):value;
  return `${fn.label} · ${source.length>42?source.slice(0,39)+'…':source || 'Value'}`;
}
