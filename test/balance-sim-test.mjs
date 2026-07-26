import fs from 'fs';
const x=new WebAssembly.Instance(new WebAssembly.Module(fs.readFileSync('site/sim.wasm')),{}).exports;
const S=()=>new Float32Array(x.memory.buffer,x.statePtr(),128);
// exploit check: park on your own castle and do nothing
x.init(101,1);
for(let f=0;f<60*240;f++){ x.setInput(0,0,0,0,0,0,0); x.step(1/60); if(S()[18]!==0)break; }
let s=S(); console.log(`park-on-castle 240s -> claimed ${(s[13]*100).toFixed(0)}% mana ${s[9].toFixed(0)} status ${s[18]} (exploit is closed if claimed stays ~0%)`);
function run(seed,lvl,secs){
  x.init(seed,lvl); let s=S(), mode='hunt';
  for(let f=0;f<60*secs;f++){
    s=S(); const px=s[0],pz=s[2],yaw=s[3],mana=s[9],cap=s[10],hp=s[7];
    if(mana>125)mode='bank'; if(mana<58)mode='hunt';
    if(hp<38)mode='bank';
    const n=x.mapCount(),M=new Float32Array(x.memory.buffer,x.mapPtr(),n*4);
    let tx,tz,bd=1e18,nh=1e18;
    for(let i=0;i<n;i++){const k=M[i*4+2];const dx=M[i*4]-px,dz=M[i*4+1]-pz,d=dx*dx+dz*dz;
      if(k===2&&d<nh)nh=d;}
    if(mode==='bank'){tx=s[33];tz=s[34];}
    else{tx=px+40;tz=pz;
      for(let i=0;i<n;i++){const k=M[i*4+2];if(k!==4&&k!==2&&k!==5)continue;
        const w=(k===4)?0.45:1.0;   // prefer loose orbs
        const dx=M[i*4]-px,dz=M[i*4+1]-pz,d=(dx*dx+dz*dz)*w;if(d<bd){bd=d;tx=M[i*4];tz=M[i*4+1];}}}
    let dy=Math.atan2(tx-px,tz-pz)-yaw; while(dy>Math.PI)dy-=2*Math.PI; while(dy<-Math.PI)dy+=2*Math.PI;
    const clr=s[23];
    x.setInput(1,0,clr<26?1:(clr>52?-0.7:0),Math.max(-0.05,Math.min(0.05,dy*0.10)),0,0,0);
    if(nh<150*150&&mana>62&&f%34===0)x.cast(0);
    if(hp<62&&s[60+7]>0&&mana>20)x.cast(7);
    if(hp<48&&s[60+6]>0&&mana>18)x.cast(6);
    x.step(1/60);
    if(S()[18]!==0){s=S();return{t:f/60,st:s[18],me:s[13],rv:s[14],lv:s[16],k:s[24]};}
  }
  s=S();return{t:secs,st:0,me:s[13],rv:s[14],lv:s[16],k:s[24]};
}
const nm={0:'timeout',1:'WIN',2:'died',3:'rival won'};
console.log('');
for(const [sd,lv] of [[31337,1],[555,1],[777,2],[9001,3],[2024,6],[4242,8],[13,10]]){
  const r=run(sd,lv,900);
  console.log(`lvl${String(lv).padStart(2)} seed${String(sd).padStart(5)}: t=${r.t.toFixed(0).padStart(3)}s ${nm[r.st].padEnd(9)} me=${(r.me*100).toFixed(0).padStart(3)}% rival=${(r.rv*100).toFixed(0).padStart(3)}% castle=${r.lv} kills=${r.k}`);
}
