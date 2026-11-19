// Browser fixture only. Native rule verification belongs to Rust tests.
import { parseVinylPosition } from './vinyl.js';

export function mockNumberingEdits(paths,read,group,options) {
  paths=[...new Set(paths)];
  if(paths.length>100000)throw new Error('Select at most 100,000 files.');
  if(options.kind==='vinyl')return mockVinylNumberingEdits(paths,read,options.source,options.keepPosition,{tracks:options.trackTotal,discs:options.discTotal});
  if(!['tracks','disc'].includes(options.kind))throw new Error('Choose a numbering action.');
  if(options.kind==='disc'&&!/^\d+$/.test(String(options.start).trim()))throw new Error('Enter a whole starting disc number.');
  const start=Number(options.start);
  if(!Number.isSafeInteger(start)||start<1)throw new Error('Enter a whole starting number of 1 or more.');
  const records=[],maxima=new Map();
  let number=start,segment=0,lastAlbum,lastGroup;
  for(const path of paths){
    const album=read(path,'album')??'',key=group(path)??'';
    const first=records.length===0,albumChanged=!first&&album!==lastAlbum,groupChanged=!first&&key!==lastGroup;
    if(!first){
      if(options.resetAlbum&&albumChanged){number=start;segment++;}
      else if(options.kind==='tracks'){
        if(options.restartGroup&&groupChanged){number=start;segment++;}else number++;
      }else if(options.advanceGroup&&groupChanged)number++;
    }
    if(!Number.isSafeInteger(number))throw new Error('The number sequence is too large.');
    records.push({path,number,segment});maxima.set(segment,Math.max(maxima.get(segment)||0,number));
    lastAlbum=album;lastGroup=key;
  }
  const field=options.kind==='tracks'?'track':'disc',totalField=options.kind==='tracks'?'tracktotal':'disctotal';
  const total=options.kind==='tracks'?options.trackTotal:options.discTotal;
  const edits=records.flatMap(row=>[{path:row.path,field,value:String(row.number)},...(total?[{path:row.path,field:totalField,value:String(maxima.get(row.segment))}]:[])]);
  return {edits,matched:records.length,skipped:0,rows:records.slice(0,3).map(row=>({path:row.path,values:[{field,label:field==='track'?'Track':'Disc',value:total?`${row.number}/${maxima.get(row.segment)}`:String(row.number)}]}))};
}

export function mockVinylNumberingEdits(paths, read, source = 'track', keepPosition = true, totals = {}) {
  if (!['track', 'disc', 'custom:POSITION'].includes(source)) throw new Error('Choose the field containing A1/B2 labels.');
  const edits = [], rows = [];
  let matched = 0, release = 0, previousAlbum;
  const converted=[],discTotals=new Map(),trackTotals=new Map();
  for (const path of paths) {
    const album=read(path,'album')??'';
    if(previousAlbum!==undefined&&album!==previousAlbum)release++;
    previousAlbum=album;
    const raw = String(read(path, source) ?? '').trim();
    const parsed = parseVinylPosition(raw);
    if (!parsed) continue;
    // The shared parser uses numbers for historical callers. Refuse a value
    // beyond exact integer precision instead of staging a rounded track.
    if (!Number.isSafeInteger(Number(parsed.track ?? 1))) throw new Error('A vinyl track number is too large.');
    const track = parsed.track ?? '1';
    edits.push({path,field:'disc',value:parsed.disc},{path,field:'track',value:track});
    if (keepPosition) edits.push({path,field:'custom:POSITION',value:raw});
    matched++;
    converted.push({path,source:raw,disc:parsed.disc,track,release});
    discTotals.set(release,Math.max(discTotals.get(release)||0,Number(parsed.disc)));
    const key=`${release}:${parsed.disc}`;trackTotals.set(key,Math.max(trackTotals.get(key)||0,Number(track)));
  }
  for(const row of converted){
    const trackTotal=String(trackTotals.get(`${row.release}:${row.disc}`)),discTotal=String(discTotals.get(row.release));
    if(totals.tracks)edits.push({path:row.path,field:'tracktotal',value:trackTotal});
    if(totals.discs)edits.push({path:row.path,field:'disctotal',value:discTotal});
    if(rows.length<3)rows.push({...row,values:[{field:'disc',label:'Disc',value:totals.discs?`${row.disc}/${discTotal}`:row.disc},{field:'track',label:'Track',value:totals.tracks?`${row.track}/${trackTotal}`:row.track}]});
  }
  return {edits,rows,matched,skipped:paths.length-matched};
}

