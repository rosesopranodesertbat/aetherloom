import assert from 'node:assert/strict';
import fs from 'node:fs';

const x=new WebAssembly.Instance(new WebAssembly.Module(fs.readFileSync('site/sim.wasm')),{}).exports;
const S=()=>new Float32Array(x.memory.buffer,x.statePtr(),128);
const HZ=x.authoritativeHz(),DT=1/HZ;
const PRESENT_EVERY=4; // 32 Hz, matching browser snapshot/presentation cadence

const outcomeName={0:'in progress',1:'WIN',2:'died',3:'rival won'};
const close=(actual,expected,tolerance=0.001)=>Math.abs(actual-expected)<=tolerance;

function checkFixedTickContract(){
  assert.equal(HZ,128,'authoritative rate is a hard protocol invariant');
  x.init(0x128,1);
  assert.equal(x.simulationTick(),0);
  x.step(99); // compatibility argument must never control authoritative time
  assert.equal(x.simulationTick(),1);
  assert.ok(close(S()[32],DT,0.000001),`one step must advance exactly ${DT}s`);
  x.advanceTick();
  x.extractFrame();
  assert.equal(x.simulationTick(),2);
  assert.ok(close(S()[32],2*DT,0.000001),'fixed and split-step ABIs must agree');
  console.log(`fixed tick: ${HZ} Hz, ${(DT*1000).toFixed(4)} ms`);
}

// A cast cue is part of the consumer-owned event queue. Simulation catch-up
// steps may append to that queue, but must not erase an event before JS reads it.
function checkEventContract(){
  x.init(0x5eed,1);
  assert.equal(x.evtCount(),0,'init must start with an empty event queue');
  x.fireSelected();
  const beforeCount=x.evtCount();
  assert.ok(beforeCount>0,'fireSelected must emit a cast cue');
  const before=Array.from(new Float32Array(x.memory.buffer,x.evtPtr(),4));
  x.setInput(0,0,0,0,0,0,0);
  x.step(DT);
  assert.ok(x.evtCount()>=beforeCount,'step must preserve queued cast cues');
  assert.deepEqual(Array.from(new Float32Array(x.memory.buffer,x.evtPtr(),4)),before,
    'step must not overwrite the queued cast cue');
  x.clearEvents();
  assert.equal(x.evtCount(),0,'clearEvents must empty the event queue');
  console.log('event queue: cast cue survives step and clearEvents empties it');
}

// Realm 8 is the first target that needs an economic tier beyond the renderer's
// six-tier visual model. Keep this arithmetic visible and guarded.
function checkRealmCapacity(){
  x.init(4242,8);
  const s=S(), tier=s[16], capacity=s[55], target=s[56];
  const capacityPerTier=capacity/tier;
  const requiredTier=Math.ceil(target/capacityPerTier);
  const spellSource=fs.readFileSync('src/spells.rs','utf8');
  assert.ok(close(capacityPerTier,240),'fortress capacity must remain 240 mana per tier');
  assert.equal(requiredTier,7,'realm 8 must exercise an economic tier above the visual tier cap');
  assert.ok(requiredTier*capacityPerTier>=target,'realm 8 must have a reachable capacity');
  assert.doesNotMatch(spellSource,/tier\s*\[\s*wizard\s*\]\s*>=/,
    'Fortress casting must not impose an economic tier ceiling');
  assert.match(spellSource,/tier\s*\[\s*wizard\s*\]\s*\+=\s*1/,
    'Fortress casting must continue incrementing the economic tier');
  console.log(`realm 8 capacity: tier ${requiredTier} holds ${requiredTier*capacityPerTier} for target ${target}`);
}

// Exploit regression: remain horizontally parked above the own keep without
// claiming or attacking. Holding lift keeps this deterministic scenario alive
// for the full window instead of letting enemies end the check after 16 seconds.
function checkParkingExploit(){
  const seconds=240,totalFrames=HZ*seconds;
  x.init(101,1);
  const initial=S(), startX=initial[0], startZ=initial[2];
  let frames=0,maxDrift=0;
  while(frames<totalFrames&&S()[18]===0){
    x.setInput(0,0,1,0,0,0,1);
    x.advanceTick();
    frames++;
    if(frames%PRESENT_EVERY===0)x.extractFrame();
    const now=S();
    maxDrift=Math.max(maxDrift,Math.hypot(now[0]-startX,now[2]-startZ));
  }
  x.extractFrame();
  const s=S(), simulated=frames/HZ;
  console.log(`hover-over-castle ${simulated.toFixed(1)}s -> stored ${s[54].toFixed(1)} `+
    `progress ${(s[13]*100).toFixed(1)}% outcome ${outcomeName[s[18]]??s[18]}`);
  assert.equal(frames,totalFrames,
    `parking regression ended after ${simulated.toFixed(1)}s with outcome ${outcomeName[s[18]]??s[18]}`);
  assert.equal(s[18],0,'parking without claiming must not end the realm');
  assert.ok(maxDrift<0.01,`parking scenario drifted ${maxDrift.toFixed(3)} world units`);
  assert.ok(s[54]<0.5,'parking without claiming must not fill the fortress');
  assert.ok(s[13]<0.001,'parking without claiming must not advance realm progress');
}

checkFixedTickContract();
checkEventContract();
checkRealmCapacity();
checkParkingExploit();

// The bot plays the current loop: hunt gold orbs, Claim them so the balloons
// have something to fetch, and go home to buy a Fortress tier when the keep
// is capped or the target needs more room than it can hold.
function run(seed,lvl,secs){
  x.init(seed,lvl); let s=S();
  const claimEvery=Math.round(HZ*40/60);
  const fortressEvery=Math.round(HZ*20/60);
  const fireEvery=Math.round(HZ*34/60);
  for(let f=0;f<HZ*secs;f++){
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
    const yawStep=0.05*60/HZ;
    x.setInput(1,0,clr<26?1:(clr>52?-0.7:0),-Math.max(-yawStep,Math.min(yawStep,dy*0.10*60/HZ)),0,0,0);
    if(!goHome&&f%claimEvery===0)x.cast(9);                       // Claim
    if(goHome&&needTier&&mana>=price&&f%fortressEvery===0)x.cast(12); // Fortress
    if(nh<150*150&&mana>70&&f%fireEvery===0)x.cast(0);            // Firebolt
    if(hp<62&&s[60+7]>0&&mana>26)x.cast(7);
    if(hp<48&&s[60+6]>0&&mana>24)x.cast(6);
    x.advanceTick();
    if((f+1)%PRESENT_EVERY===0)x.extractFrame();
    if(S()[18]!==0){s=S();return{t:(f+1)/HZ,st:s[18],me:s[13],rv:s[14],tier:s[16],k:s[24],store:s[54],cap:s[55],tgt:s[56]};}
  }
  x.extractFrame();
  s=S();return{t:secs,st:0,me:s[13],rv:s[14],tier:s[16],k:s[24],store:s[54],cap:s[55],tgt:s[56]};
}
const nm={0:'timeout',1:'WIN',2:'died',3:'rival won'};
const results=[];
console.log('');
for(const [sd,lv] of [[31337,1],[555,1],[777,2],[9001,3],[2024,6],[4242,8],[13,10]]){
  const r=run(sd,lv,900);
  results.push(r);
  for(const [name,value] of Object.entries(r)) assert.ok(Number.isFinite(value),`${name} must be finite`);
  assert.ok(Number.isInteger(r.st)&&r.st>=0&&r.st<=3,`invalid outcome ${r.st}`);
  assert.ok(Number.isInteger(r.tier)&&r.tier>=1,`invalid fortress tier ${r.tier}`);
  assert.ok(close(r.cap,r.tier*240,0.01),`tier ${r.tier} capacity ${r.cap} is not 240 x tier`);
  assert.ok(r.store>=0&&r.store<=r.cap+0.01,`fortress store ${r.store} exceeds capacity ${r.cap}`);
  assert.ok(r.tgt>0,'realm target must be positive');
  assert.ok(r.me>=0&&r.rv>=0,'realm progress must not be negative');
  console.log(`lvl${String(lv).padStart(2)} seed${String(sd).padStart(5)}: t=${r.t.toFixed(0).padStart(3)}s ${nm[r.st].padEnd(9)} fortress=${r.store.toFixed(0).padStart(4)}/${r.tgt.toFixed(0)} rival=${(r.rv*100).toFixed(0).padStart(3)}% tier=${r.tier} kills=${r.k}`);
}
assert.ok(results.some((r)=>r.st===1),'scripted balance sweep must retain at least one player win');
