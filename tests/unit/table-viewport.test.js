import test from 'node:test';
import assert from 'node:assert/strict';
import { tableCoveredWidth, tableViewportBounds } from '../../app/ui/js/table-viewport.js';
function viewport(flags={sidebarDrawer:'true',sidebarClosed:'false'}) {
  const frame={dataset:flags,querySelector:()=>({clientWidth:420})};
  return {id:'files-view',clientWidth:1200,clientHeight:700,clientLeft:0,clientTop:0,closest:()=>frame,getBoundingClientRect:()=>({left:12,top:100})};
}
test('an open desktop overlay leaves a smaller interactive viewport',()=>{
  const wrap=viewport();assert.equal(tableCoveredWidth(wrap),420);
  assert.deepEqual(tableViewportBounds(wrap),{left:12,top:100,right:792,bottom:800,width:780,height:700});
});
test('collapsed, compact and unrelated scrollports retain their full bounds',()=>{
  for(const wrap of [viewport({sidebarDrawer:'true',sidebarClosed:'true'}),viewport({sidebarDrawer:'false',sidebarClosed:'false'}),{...viewport(),id:'editor-scroll'}]){
    assert.equal(tableCoveredWidth(wrap),0);assert.equal(tableViewportBounds(wrap).right,1212);
  }
});
