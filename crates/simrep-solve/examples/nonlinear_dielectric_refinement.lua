-- Run from a generated refinement directory. Reuse the original example's
-- constitutive and field routines, rather than reimplementing field_vc.
-- Usage: simion --nogui --noprompt lua <this file> <original Lua> [updates] [objective] [seed PA]
local original=assert(arg[1],'expected original sphere_nonlinear_3dp.lua')
local updates=tonumber(arg[2]) or 50
local objective=tonumber(arg[3]) or 1e-7
local pa=simion.pas:open(arg[4] or 'potential.pa')
local base=simion.pas:open('epsilon.pa')
local epsilon=simion.pas:open('epsilon2.pa')
local h=pa.dx_mm
local cells={}
for x,y,z in base:points() do
  if base:potential(x,y,z)==6 then cells[#cells+1]={x,y,z} end
end
local file=assert(io.open(original,'r'))
local source=file:read('*a')
file:close()
source=assert(source:match('(%-%- Defines non%-linear dielectric properties.-)%-%- called by SIMION'))
source=source:gsub('local X0,Y0,Z0 = 100, 0, 0','local X0,Y0,Z0 = '..(50/h)..', 0, 0')
source=source:gsub('r >= 20 and r < 21','r >= '..(10/h)..' and r < '..(10/h+1))
source=source:gsub('local s = 21/r','local s = '..(10/h+1)..'/r')
source=source:gsub('r < 20 and r > 19','r < '..(10/h)..' and r > '..(10/h-1))
source=source:gsub('local s = 19/r','local s = '..(10/h-1)..'/r')
-- Vacuum writes are identically one. Only strict refinement, not speed
-- measurements, elides them. The original field/update equations are retained.
source=source:gsub('for xg,yg,zg in di2pa:points%(%) do',
  'for _,c in ipairs(cells) do local xg,yg,zg=c[1],c[2],c[3]')
source=source:gsub("if xg==100 and yg==0 and zg==0 then print%('k%(center%)=', k%) end","")
local update=assert(loadstring('local pa,dipa,di2pa,painst,cells=...; '..source..'\nreturn update_dielectrics'))(
  pa,base,epsilon,{pa=pa,scale=1},cells)
if arg[4] then update() end
local previous={}
local start=os.clock()
for iteration=1,updates do
  pa:refine{permittivity=epsilon,convergence=objective,skipped_point=false}
  update()
  local delta=0
  for i,c in ipairs(cells) do
    local k=epsilon:potential(c[1],c[2],c[3])
    if previous[i] then delta=math.max(delta,math.abs(k-previous[i])) else delta=math.huge end
    previous[i]=k
  end
  print(string.format('REFINEMENT_SIMION h=%.9g iteration=%d delta_epsilon=%.12e elapsed=%.9f',
    h,iteration,delta,os.clock()-start))
  if iteration==updates-5 then pa:save('simion-check.pa') end
end
pa:save('simion.pa')
epsilon:save('simion-epsilon.pa')
print('REFINEMENT_SIMION_DONE')
