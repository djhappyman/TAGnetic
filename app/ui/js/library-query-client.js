// Native snapshots keep the large library out of per-keystroke IPC and Svelte state.
export function createLibraryQueryClient(invoke) {
  let queryRevision=0,suggestRevision=0,snapshotId=null,paths=[];
  const manifests=new Map();
  function accept(result) {
    if(result.paths) {
      manifests.set(result.snapshotId,result.paths);
      if(manifests.size>3) manifests.delete(manifests.keys().next().value);
    }
    const manifest=manifests.get(result.snapshotId);
    if(!manifest) throw new Error('The library changed. Retry the search.');
    snapshotId=result.snapshotId;paths=manifest;
    return {...result,matchedPaths:result.indices.map(i=>manifest[i]),groups:(result.groups||[]).map(g=>({...g,paths:g.indices.map(i=>manifest[i])}))};
  }
  return {
    async run(query) {
      const requestId=++queryRevision;
      const result=await invoke('library_query',{request:{requestId,snapshotId,query}});
      if(requestId!==queryRevision||result.cancelled) return null;
      return accept(result);
    },
    async suggest(query,text,field) {
      const requestId=++suggestRevision;
      const result=await invoke('library_suggest',{request:{requestId,snapshotId,query,text,field,limit:12,strategy:'fields'}});
      return requestId===suggestRevision&&!result.cancelled?result.suggestions:null;
    },
    invalidate() {queryRevision++;suggestRevision++;snapshotId=null;paths=[];manifests.clear();},
    cancel() {queryRevision++;suggestRevision++;},
    cancelSuggestions() {suggestRevision++;},
    get snapshotId(){return snapshotId;},
    get paths(){return paths;},
  };
}
