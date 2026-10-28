import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, writeFile, readFile, stat, rm } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { workflowTools } from '../../mcp/workflows.mjs'

test('optimizer uses analyzer_scan and carries sampling warnings into history', async t => {
  const dir=await mkdtemp(path.join(os.tmpdir(),'simrep-opt-analyzer-')); t.after(()=>rm(dir,{recursive:true,force:true}))
  await writeFile(path.join(dir,'template.gem'),'{{x}}')
  const insideWorkspace = p => { const full=path.resolve(dir,p); if (!full.startsWith(dir+path.sep)) throw new Error('outside workspace'); return full }
  const inputFile = p => insideWorkspace(p)
  const prepareOutput = p => insideWorkspace(p)
  let tools
  const runTool = async (name,args) => {
    if (name === 'refine_geometry') { await writeFile(args.out+'.pa0','mock'); return { text:JSON.stringify({converged:true,compute_ms:1}) } }
    if (name === 'fly') { const v=Number(args.voltage.at(-1).split('=')[1]); await writeFile(args.output,`ion,x_mm,y_mm,z_mm,reason\n1,0,0,0,${v===2?'plane':'outside'}\n`); return {text:JSON.stringify({ion_count:1,compute_ms:1})} }
    return tools.find(t=>t.name===name).run(args)
  }
  tools=workflowTools({workspace:dir,inputFile,insideWorkspace,prepareOutput,runTool,timeoutSeconds:20})
  const result=JSON.parse((await tools.find(t=>t.name==='optimize_design').run({template:'template.gem',parameters:[{name:'x',min:0,max:1,initial:0.5}],electrodes:['1=1:0'],out:'run',budget:1,objective:{kind:'analyzer',energies_ev:[1],channel_voltages:[1,2,3],electrode:1,beam:{},plane:[1,0,0,1],aperture:[{axis:'y',center_mm:0,width_mm:1}],duration_us:1,bootstrap:false,targets:{k_v_per_ev:2,de_over_e:0.5},weights:{k_v_per_ev:1,de_over_e:1}}})).text)
  assert.equal(result.history[0].objective,0)
  assert.ok(result.history[0].warnings.some(w=>w.includes('fewer than 30')))
  assert.equal(result.history[0].compute_ms,4)
})
