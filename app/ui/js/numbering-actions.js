// A shared native-plan boundary for the three independent numbering actions.
export function createNumberingActions({captureSelection,isSelectionCurrent,getPlan,inputRevision,
  read,group,order,context,isFieldLocked,invoke,adoptTagOnlyPreview,recordLiveFields,writeQuickPlan}){
  let generation=0,accepted=null;
  const current=item=>{
    if(item.base!==getPlan()||item.revision!==inputRevision()||item.context!==context()||!isSelectionCurrent(item.subject)||item.fields.some(isFieldLocked))return false;
    const now=order(item.subject.paths);return now.length===item.paths.length&&now.every((path,i)=>path===item.paths[i]&&String(group(path)??'')===item.groups[i]);
  };
  return async(action,options={})=>{
    if(action==='numbering-cancel'){generation++;accepted=null;return;}
    if(action==='numbering-preview'){
      const token=++generation;accepted=null;
      const subject=captureSelection(),paths=[...new Set(order(subject.paths))];
      const native=numberingOptions(options),fields=numberingFields(native);
      const sources=native.kind==='vinyl'?['album',native.source]:native.kind==='disc'&&native.resetAlbum?['album']:[];
      // Snapshot current presentation inputs, including raw edits still awaiting
      // stage acknowledgement. Only Rust calculates the resulting assignments.
      const working=sources.length?{description:'Numbering inputs',changes:paths.map(path=>({
        path,rename_to:null,tag_changes:sources.map(field=>{const input=read(path,field);return {field,old:input.saved||null,new:input.value||null};}),
      }))}:null;
      if(fields.some(isFieldLocked))throw new Error('Unlock the numbering fields before continuing.');
      const groups=paths.map(path=>String(group(path)??''));
      const item={token,subject,paths,groups,fields,base:getPlan(),revision:inputRevision(),context:context()};
      const result=await invoke('preview_numbering_dialog',{paths,groups,options:native,working});
      const plan=result.plan;
      if(token!==generation||!current(item))throw new Error('The files, order or staged values changed. Preview again.');
      const invalid=!!plan.locked_skipped?.length||plan.changes.some(change=>change.tag_changes?.some(edit=>edit.invalid));
      accepted={...item,plan,edits:result.edits,invalid};
      return {token,matched:result.matched,skipped:result.skipped,rows:[],invalid};
    }
    const item=accepted;
    if(!item||item.token!==options.token||!current(item))throw new Error('The files, order or staged values changed. Preview again.');
    if(item.invalid)throw new Error('These numbers are not supported by the selected files.');
    if(action==='numbering-write')await writeQuickPlan(item.plan);
    else if(action==='numbering-stage'){adoptTagOnlyPreview();recordLiveFields(item.edits);}
    else throw new Error('Unknown numbering action.');
    generation++;accepted=null;return {dialog:'ribbon-number-dialog'};
  };
}

// Serialize active controls only; inactive tabs cannot affect another action.
export function numberingOptions(options){
  if(options.kind==='tracks')return {kind:'tracks',start:String(options.start),trackTotal:!!options.trackTotal,restartGroup:!!options.restartGroup};
  if(options.kind==='disc')return {kind:'disc',start:String(options.start),discTotal:!!options.discTotal,advanceGroup:!!options.advanceGroup,resetAlbum:!!options.resetAlbum};
  if(options.kind==='vinyl')return {kind:'vinyl',source:options.source,keepPosition:!!options.keepPosition,trackTotal:!!options.trackTotal,discTotal:!!options.discTotal};
  throw new Error('Choose a numbering action.');
}
function numberingFields(options){
  const fields=options.kind==='vinyl'?['track','disc']:options.kind==='tracks'?['track']:['disc'];
  if(options.trackTotal)fields.push('tracktotal');
  if(options.discTotal)fields.push('disctotal');
  if(options.kind==='vinyl'&&options.keepPosition)fields.push('custom:POSITION');
  return fields;
}
