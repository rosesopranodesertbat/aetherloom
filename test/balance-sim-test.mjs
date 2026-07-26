import fs from 'fs';
const x=new WebAssembly.Instance(new WebAssembly.Module(fs.readFileSync('site/sim.wasm')),{}).exports;
const S=()=>new Float32Array(x.memory.buffer,x.statePtr(),128);
// exploit check: park on your own castle and do nothing
x.init(101,1);
for(let f=0;f<60*240;f++){ x.setInput(0,0,0,0,0,0,0); x.step(1/60); if(S()[18]!==0)break; }
let s=S(); console.log(`park-on-castle 240s -> claimed ${(s[13]*100).toFixed(0)}% mana ${s[9].toFixed(0)} status ${s[18]} (exploit is closed if claimed stays ~0%)`);
// The bot plays the current loop: hunt gold orbs, Claim them so the balloons
// have something to fetch, and go home to buy a Fortress tier when the keep
// is capped or the target needs more room than it can hold.
function run(seed,lvl,secs){
  x.init(seed,lvl); let s=S();
  for(let f=0;f<60*secs;f++){
    s=S();
    const px=s[0],pz=s[2],yaw=s[3],mana=s[9],hp=s[7];
    const store=s[54],cap=s[55],tgt=s[56],tier=s[16],price=s[112];
    const n=x.mapCount(),M=new Float32Array(x.memory.buffer,x.mapPtr(),n*4);
    // gold orbs are kind 10, wildlife 2, nests 5
    let gx=null,gz=null,bd=1e18,nh=1e18;
    for(let i=0;i<n;i++){
      const k=M[i*4+2],dx=M[i*4]-px,dz=M[i*4+1]-pz,d=dx*dx+dz*dz;
      if(k===2&&d<nh)nh=d;
      if((k===10||k===2||k===5)&&d*(k===10?0.4:1.0)<bd){bd=d*(k===10?0.4:1.0);gx=M[i*4];gz=M[i*4+1];}
    }
    // go home to upgrade when capped (or nearly) and we can pay for it
    const needTier = cap < tgt;
    // buy capacity ahead of the fill, not once it is already jammed
    const goHome = hp<38 || (needTier && mana>=price) || gx===null;
    const tx = goHome ? s[33] : gx, tz = goHome ? s[34] : gz;
    let dy=Math.atan2(tx-px,tz-pz)-yaw; while(dy>Math.PI)dy-=2*Math.PI; while(dy<-Math.PI)dy+=2*Math.PI;
    const clr=s[23];
    x.setInput(1,0,clr<26?1:(clr>52?-0.7:0),-Math.max(-0.05,Math.min(0.05,dy*0.10)),0,0,0);
    if(!goHome&&f%40===0)x.cast(9);                       // Claim
    if(goHome&&needTier&&mana>=price&&f%20===0)x.cast(12); // Fortress
    if(nh<150*150&&mana>70&&f%34===0)x.cast(0);            // Firebolt
    if(hp<62&&s[60+7]>0&&mana>26)x.cast(7);
    if(hp<48&&s[60+6]>0&&mana>24)x.cast(6);
    x.step(1/60);
    if(S()[18]!==0){s=S();return{t:f/60,st:s[18],me:s[13],rv:s[14],lv:s[16],k:s[24],store:s[54],tgt:s[56]};}
  }
  s=S();return{t:secs,st:0,me:s[13],rv:s[14],lv:s[16],k:s[24],store:s[54],tgt:s[56]};
}
const nm={0:'timeout',1:'WIN',2:'died',3:'rival won'};
console.log('');
for(const [sd,lv] of [[31337,1],[555,1],[777,2],[9001,3],[2024,6],[4242,8],[13,10]]){
  const r=run(sd,lv,900);
  console.log(`lvl${String(lv).padStart(2)} seed${String(sd).padStart(5)}: t=${r.t.toFixed(0).padStart(3)}s ${nm[r.st].padEnd(9)} fortress=${r.store.toFixed(0).padStart(4)}/${r.tgt.toFixed(0)} rival=${(r.rv*100).toFixed(0).padStart(3)}% tier=${r.lv} kills=${r.k}`);
}
