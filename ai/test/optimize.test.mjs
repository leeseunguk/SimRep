import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, mkdir, readFile, writeFile, chmod, rm, symlink } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { spawn } from 'node:child_process'
import readline from 'node:readline'
import { fileURLToPath } from 'node:url'

const server = fileURLToPath(new URL('../../mcp/server.mjs', import.meta.url))

test('MCP optimization finds a minimum, records infeasibility, resumes and rejects injection', { timeout: 60000 }, async t => {
  const dir = await mkdtemp(path.join(os.tmpdir(), 'simrep-opt-')); t.after(() => rm(dir, { recursive: true, force: true }))
  const bin = path.join(dir, 'mock.mjs')
  await writeFile(bin, `#!/usr/bin/env node\nimport fs from 'node:fs';\nconst a=process.argv.slice(2), get=k=>a[a.indexOf(k)+1];\nif(a.includes('refine-geometry')) { const x=Number(fs.readFileSync(a[a.indexOf('refine-geometry')+1],'utf8')); if(x<0) { console.log(JSON.stringify({error:{message:'gap check failed'}})); process.exit(1) }; fs.writeFileSync(get('--out')+'.pa0',String(x)); console.log(JSON.stringify({converged:true,compute_ms:1,warnings:['mock warning']})); } else if(a.includes('fly')) { const x=Number(fs.readFileSync(get('--pa'),'utf8')); console.log(JSON.stringify({arrivals:1,arrival_tof_us:{mean:(x-2)**2},compute_ms:2})); }\n`)
  await chmod(bin, 0o755); await writeFile(path.join(dir, 'template.gem'), '{{x}}')
  const proc = spawn(process.execPath, [server], { env: { ...process.env, SIMREP_WORKSPACE: dir, SIMREP_BIN: bin }, stdio: ['pipe','pipe','pipe'] }); t.after(() => proc.kill())
  let id = 0; const pending = new Map(); readline.createInterface({ input: proc.stdout }).on('line', line => { const m = JSON.parse(line); const done = pending.get(m.id); if (done) { pending.delete(m.id); done(m) } })
  const call = (name, args) => new Promise((resolve, reject) => { const i=++id; const timer=setTimeout(() => reject(new Error('MCP timeout')), 30000); pending.set(i, m => { clearTimeout(timer); resolve(m.result) }); proc.stdin.write(JSON.stringify({jsonrpc:'2.0',id:i,method:'tools/call',params:{name,arguments:args}})+'\n') })
  const args = { template:'template.gem', parameters:[{name:'x',min:-1,max:4,initial:3,step:1}], electrodes:['1=1:0'], objective:{kind:'flight_scalar',key:'arrival_tof_us.mean',fly:{particles:'ions.ion',plane:[1,0,0,1],duration_us:1}}, out:'search', budget:3, xtol:0.01, ftol:0.0001 }
  await writeFile(path.join(dir,'ions.ion'), 'fixture')
  const invoke = async a => { const r=await call('optimize_design',a); assert.equal(r.isError,false, r.content[0].text); return JSON.parse(r.content[0].text) }
  const small = await invoke(args); assert.equal(small.verdict,'budget_exhausted'); assert.equal(small.history.length,3)
  const resumed = await invoke({...args,budget:60}); assert.equal(resumed.converged,true); assert.ok(Math.abs(resumed.best_parameters.x-2)<0.02); assert.ok(resumed.best_objective < 0.0001); assert.deepEqual(resumed.history.slice(0,3),small.history)
  const grid = await invoke({...args,out:'grid',budget:10,method:'grid'}); assert.equal(grid.history[0].objective,'+inf'); assert.match(grid.history[0].reason,/gap check/); assert.ok(grid.history[0].warnings.length)
  const mismatch = await call('optimize_design',{...args,budget:60,xtol:0.1}); assert.equal(mismatch.isError,true)
  const escaped = await call('optimize_design',{...args,out:'../escape'}); assert.equal(escaped.isError,true)
  await mkdir(path.join(dir,'symlink','eval-0001'),{recursive:true}); await symlink('/tmp',path.join(dir,'symlink','eval-0001','evil')); const linked=await call('optimize_design',{...args,out:'symlink'}); assert.equal(linked.isError,true)
  const rejected = await call('optimize_design',{...args,out:'bad',parameters:[{...args.parameters[0],initial:'2); dangerous()'}]}); assert.equal(rejected.isError,true)
  await writeFile(path.join(dir,'bad.gem'),'{{x}} {{x;evil}}'); const injection=await call('optimize_design',{...args,template:'bad.gem',out:'bad'}); assert.equal(injection.isError,true)
  const history=JSON.parse(await readFile(path.join(dir,'search','history.json'),'utf8')); assert.equal(history.history.length,resumed.history.length)
})
