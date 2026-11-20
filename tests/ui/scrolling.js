// Wait for native wheel momentum before asserting that another action preserves
// the offset. A fixed delay can still sample a moving WebKit scroll animation.
export async function waitForScrollIdle(scrollport) {
  await scrollport.evaluate(async element => {
    let x=element.scrollLeft, y=element.scrollTop, stable=0;
    const deadline=performance.now()+5000;
    while (stable<6 && performance.now()<deadline) {
      await new Promise(requestAnimationFrame);
      const nextX=element.scrollLeft, nextY=element.scrollTop;
      stable=Math.abs(nextX-x)<.5 && Math.abs(nextY-y)<.5 ? stable+1 : 0;
      x=nextX; y=nextY;
    }
    if (stable<6) throw new Error('Native scrolling did not settle');
  });
}
