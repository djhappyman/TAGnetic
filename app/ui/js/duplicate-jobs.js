// Read-only scan lifecycle. Expensive work and analysis caches remain native.
export function createDuplicateJobs(invoke, onState, onComplete) {
  let generation=0,jobId=null,timer=null;
  async function publish(result,version) {
    if(version!==generation)return;
    onState(result);
    if(result.status==='running')timer=setTimeout(async()=>{
      try{await publish(await invoke('library_duplicates_status',{jobId}),version);}
      catch(error){if(version===generation)onState({status:'error',error:error.message});}
    },180);
    else onComplete(result);
  }
  return {
    async start(kind) {
      clearTimeout(timer);const version=++generation;
      if(jobId!=null)await invoke('library_duplicates_cancel',{jobId}).catch(()=>{});
      if(version!==generation)return;
      onState({status:'running',processed:0,total:0,skipped:0,errors:0});
      try{const result=await invoke('library_duplicates_start',{kind});if(version!==generation){await invoke('library_duplicates_cancel',{jobId:result.jobId}).catch(()=>{});return;}jobId=result.jobId;await publish(result,version);}
      catch(error){if(version===generation)onState({status:'error',error:error.message});}
    },
    async cancel(){clearTimeout(timer);const version=++generation;const previous=jobId;jobId=null;const result=previous!=null?await invoke('library_duplicates_cancel',{jobId:previous}).catch(()=>null):null;if(version===generation)onState({...result,status:'cancelled'});},
    invalidate(){clearTimeout(timer);generation++;jobId=null;},
  };
}
