// Shared deterministic workloads for Node/Rust and actual WebKit/IPC measurements.
const leaf=(id,field,operator,value)=>({id,field,operator,value});
export const workloads=[
 {name:'Free text',old:{text:'Artist 17'}},
 {name:'Genre + missing artwork',old:{text:'genre:Hardcore',conditions:[leaf('cover','cover','equals','missing')]}},
 {name:'Artist equality + artwork',old:{conditions:[leaf('artist','artist','equals','Artist 17'),leaf('cover','cover','equals','embedded')]}},
 {name:'Alternative conditions + text',old:{text:'genre:Jungle',mode:'any',conditions:[leaf('artist','artist','contains','Artist 1'),leaf('title','title','starts','Track 9')]}},
 {name:'Bounded regex',old:{text:'artist:^Artist (17|170)$',regex:true}},
];
export const oldModel=w=>({version:1,text:'',regex:false,caseSensitive:false,conditions:[],mode:'all',...w.old});
export const nativeModel=w=>{const q=oldModel(w);return {version:2,text:q.text,regex:q.regex,caseSensitive:q.caseSensitive,mode:'search',duplicateKind:'exact',root:{kind:'group',id:'root',mode:q.mode,children:q.conditions.map(c=>({kind:'condition',id:c.id,field:c.field,operator:c.operator,values:[c.value],negate:false}))}};};
export function fixture(size){const genres=['Hardcore','Jungle','House','Techno','Drum & Bass','Trance','Breakbeat','Ambient'];return Array.from({length:size},(_,i)=>({path:`/virtual/Album ${Math.floor(i/10)}/${String(i).padStart(6,'0')}.mp3`,format:i%5?'Mp3':'Flac',unreadable:i%211===0,has_cover:i%3===0,duration_secs:180+i%400,tag_blocks:[],tags:{artist:`Artist ${i%1000}`,title:`Track ${i}`,album:`Release ${Math.floor(i/10)}`,genre:genres[i%8],year:String(1990+i%35),label:`Label ${i%100}`,comment:i%7?'':'Needs review'}}));}
