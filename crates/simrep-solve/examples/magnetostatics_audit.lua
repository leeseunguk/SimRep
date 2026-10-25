-- Run one case and mode per bounded SIMION process in a writable staging cwd.
-- Arguments: installed-example-directory, example-stem, default|strict, reps.
-- Only generated copies are saved. No flight or plotting callbacks are run.
local root=assert(arg[1])
local name=assert(arg[2])
local mode=arg[3] or 'default'
local repetitions=tonumber(arg[4]) or 3
-- SIMION rejects objectives below 1e-7. Linearity permits a stricter physical
-- reference: scale poles and sources together, refine, then undo the scale.
local strict_scale=mode=='strict' and (
  name=='current_sphere_3dp' and 100 or name=='mag_sphere_3dp' and 1 or 1000) or 1
assert(mode=='default' or mode=='strict')
assert(repetitions>=1)
simion.early_access(8.2)
local definitions={
  c_magnet_2dp={'mu','jz','Az'},
  current_cylinder_2dp={'jz','Az'},
  current_sphere_2dc={'jz','psi'},
  current_sphere_3dp={'Ax','Ay','Az','jx','jy','jz'},
  emag_quad_2dp={'mu','jz','Az'},
  mag_2cylinder_2dc={'Mx','j','base'},
  mag_2ring_2dc={'Mx','j','base'},
  mag_cylinder_2dc={'j','om','base'},
  mag_halbach_cylinder_2dp={'Mx','My','j','base'},
  mag_quad_2dp={'Mx','My','j','mu','base'},
  mag_ring_2dc={'Mx','j','base'},
  mag_sphere_2dc={'j','base'},
  mag_sphere_3dp={'j','base'},
  mu_cylinder_2dp={'mu','Az','base'},
  mu_sphere_2dc={'mu','Az','base'},
  mu_sphere_3dp={'mu','base'},
  mu_torus_3dp={'mu','base'},
  mucurrent_cylinder_2dp={'mu','jz','Az'},
  solenoid_2dc={'jz','mu','psi'},
  uniform_2dp={'Az','base'},
}
local suffixes=assert(definitions[name],'not a PDE example')
local function filename(suffix,extension)
  return name..(suffix=='base' and '' or '-'..suffix)..'.'..extension
end
local function copy(source,dest)
  local f=assert(io.open(source,'rb'));local content=f:read('*a');f:close()
  f=assert(io.open(dest,'wb'));assert(f:write(content));assert(f:close())
end
if mode=='default' then
  _G.gap=20 -- C-magnet GEM fallback is 30; Lua default is 20.
  for _,suffix in ipairs(suffixes) do
    local gem=filename(suffix,'gem')
    copy(root..'/'..gem,gem)
    simion.command('gem2pa "'..gem..'"')
  end
  copy(root..'/'..name..'.iob',name..'.iob')
  copy(root..'/'..name..'.lua',name..'.lua')
end
simion.pas:close()
simion.wb:load(name..'.iob')
local pa,inst={},{}
for i,suffix in ipairs(suffixes) do
  inst[suffix]=simion.wb.instances[i]
  pa[suffix]=inst[suffix].pa
  local f=assert(io.open(name..'.transforms.csv',i==1 and 'w' or 'a'))
  f:write(suffix)
  for _,q in ipairs({{0,0,0},{1,0,0},{0,1,0},{0,0,1}}) do
    local x,y,z=inst[suffix]:pa_to_wb_coords(unpack(q))
    f:write(string.format(',%.17g,%.17g,%.17g',x,y,z))
  end
  f:write('\n');f:close()
end
local function replace_mu(inside)
  for x,y,z in pa.mu:points() do
    pa.mu:potential(x,y,z,pa.mu:potential(x,y,z)==1 and 1 or inside)
  end
end
if name=='c_magnet_2dp' then
  replace_mu(1000)
  local area=0
  for x,y,z in pa.jz:points() do
    if pa.jz:potential(x,y,z)==1 then
      area=area+pa.jz.dx_mm*pa.jz.dy_mm*inst.jz.scale^2
    end
  end
  assert(area>0);pa.jz:potentials_scale(0,2000/area)
elseif name=='current_cylinder_2dp' then
  for x,y,z in pa.jz:points() do
    if ((x+.5)*pa.Az.dx_mm)^2+((y+.5)*pa.Az.dy_mm)^2<=20^2 then
      pa.jz:potential(x,y,z,1/(math.pi*20^2))
    end
  end
  for x,y,z in pa.Az:points() do pa.Az:point(x,y,z,0,x==pa.Az.nx-1 or y==pa.Az.ny-1) end
elseif name=='current_sphere_2dc' then
  for x,y,z in pa.jz:points() do
    local value=0
    if (x+.5)^2+(y+.5)^2+(z+.5)^2<=20^2 then value=1000*(y+.5)/(4/3*math.pi*20^3) end
    pa.jz:potential(x,y,z,value)
  end
  for x,y,z in pa.psi:points() do pa.psi:point(x,y,z,0,x==pa.psi.nx-1 or y==0 or y==pa.psi.ny-1) end
elseif name=='current_sphere_3dp' then
  for x,y,z in pa.jx:points() do
    local jx,jy=0,0
    if (x+.5)^2+(y+.5)^2+(z+.5)^2<=20^2 then
      local density=1e6/(4/3*math.pi*20^3)
      jx,jy=-density*(y+.5),density*(x+.5)
    end
    pa.jx:potential(x,y,z,jx);pa.jy:potential(x,y,z,jy);pa.jz:potential(x,y,z,0)
  end
  for x,y,z in pa.Ax:points() do
    local far=x==pa.Ax.nx-1 or y==pa.Ax.ny-1 or z==pa.Ax.nz-1
    pa.Ax:point(x,y,z,0,far or y==0)
    pa.Ay:point(x,y,z,0,far or x==0)
    pa.Az:point(x,y,z,0,far)
  end
elseif name=='mucurrent_cylinder_2dp' then
  replace_mu(5)
  for x,y,z in pa.jz:points() do
    local inside=((x+.5)*.5)^2+((y-200+.5)*.5)^2<=10^2
    pa.jz:potential(x,y,z,inside and 100/(math.pi*10^2) or 0)
  end
elseif name=='solenoid_2dc' then
  replace_mu(10)
elseif name=='mu_sphere_2dc' then
  replace_mu(10)
elseif name=='mu_cylinder_2dp' or name=='mu_sphere_3dp' then
  replace_mu(5)
end
if name=='mag_2cylinder_2dc' or name=='mag_2ring_2dc' or name=='mag_ring_2dc'
    or name=='mag_halbach_cylinder_2dp' or name=='mag_quad_2dp' then
  for x,y,z in pa.j:points() do
    local dx=pa.Mx:field_vc(x,y,z)
    local dy=0
    if pa.My then local unused;unused,dy=pa.My:field_vc(x,y,z) end
    pa.j:potential(x,y,z,(dx+dy)/pa.Mx.ng)
  end
end
for suffix,array in pairs(pa) do array:save(filename(suffix,'input.pa')) end
local jobs={}
local function job(suffix,kind,charge,mu,objective,anti)
  jobs[#jobs+1]={suffix=suffix,kind=kind,charge=charge,mu=mu,objective=objective,anti=anti or ''}
end
if name=='current_sphere_3dp' then
  job('Ax','cartesian_a','jx',nil,1e-4,'y')
  job('Ay','cartesian_a','jy',nil,1e-4,'x')
  job('Az','cartesian_a','jz',nil,1e-4,'xy')
elseif name=='mu_cylinder_2dp' or name=='mu_sphere_2dc' then
  job('base','scalar',nil,'mu',1e-5,'x')
  job('Az',name=='mu_sphere_2dc' and 'axisymmetric_psi' or 'planar_az',nil,'mu',1e-5,
      name=='mu_cylinder_2dp' and 'y' or '')
elseif name=='uniform_2dp' then
  job('Az','planar_az',nil,nil,1e-5)
  job('base','scalar',nil,nil,1e-5)
  -- Scalar is refined while the workbench loads, not by the shipped segment.
  pa.base:save(filename('base',mode..'.pa'))
elseif pa.psi then
  job('psi','axisymmetric_psi','jz',pa.mu and 'mu',1e-4)
elseif pa.Az and not pa.base then
  job('Az','planar_az','jz',pa.mu and 'mu',1e-5,name=='emag_quad_2dp' and 'xy' or '')
else
  job('base','scalar',pa.j and 'j',pa.mu and 'mu',
      name:sub(1,4)=='mag_' and 1e-7 or 1e-5,
      (name:sub(1,10)=='mag_sphere' or name:sub(1,3)=='mu_') and 'x'
      or name=='mag_halbach_cylinder_2dp' and 'y' or '')
end
local manifest=assert(io.open(name..'.jobs.csv','w'))
local times=assert(io.open(name..'.'..mode..'.times.csv','w'))
local cartesian,cartesian_current
if name=='current_sphere_3dp' then
  cartesian=simion.experimental.field_array{Ax=inst.Ax,Ay=inst.Ay,Az=inst.Az,antimirror='xy'}
  cartesian_current=simion.experimental.field_array{inst.jx,inst.jy,inst.jz,antimirror='xy'}
end
for _,j in ipairs(jobs) do
  manifest:write(table.concat({j.suffix,j.kind,j.charge or '',j.mu or '',j.anti},',')..'\n')
  local options={charge=j.charge and pa[j.charge],permeability=j.mu and pa[j.mu],
    convergence=mode=='strict' and 1e-7 or j.objective,max_iterations=100000}
  if mode=='strict' then options.skipped_point=false end
  if j.kind=='axisymmetric_psi' then options.potential_type='magnetic[r*A]'
  elseif j.kind~='scalar' and name~='current_cylinder_2dp' then options.potential_type='magnetic[A]' end
  local load_refined=name=='uniform_2dp' and j.suffix=='base'
  for repetition=1,((cartesian and j.suffix~='Ax') or load_refined) and 0 or repetitions do
    pa[j.suffix]:load(filename(j.suffix,'input.pa'))
    if j.mu then pa[j.mu]:load(filename(j.mu,'input.pa')) end
    if cartesian then
      pa.Ay:load(filename('Ay','input.pa'));pa.Az:load(filename('Az','input.pa'))
      options.charge=cartesian_current
    end
    local targets=cartesian and {'Ax','Ay','Az'} or {j.suffix}
    local sources=cartesian and {'jx','jy','jz'} or (j.charge and {j.charge} or {})
    for _,suffix in ipairs(targets) do pa[suffix]:potentials_scale(0,strict_scale) end
    for _,suffix in ipairs(sources) do
      pa[suffix]:load(filename(suffix,'input.pa'))
      pa[suffix]:potentials_scale(0,strict_scale)
    end
    local start=os.clock()
    local error
    if cartesian then error=cartesian:refine(options) or 0
    else error=pa[j.suffix]:refine(options) end
    local elapsed=os.clock()-start
    for _,suffix in ipairs(targets) do pa[suffix]:potentials_scale(0,1/strict_scale) end
    error=error/strict_scale
    times:write(string.format('%s,%d,%.9f,%.17g\n',j.suffix,repetition,elapsed,error))
    times:flush()
    print(string.format('MAGNETIC_AUDIT,%s,%s,%s,%d,%.9f,%.17g',name,j.suffix,mode,repetition,elapsed,error))
  end
  pa[j.suffix]:save(filename(j.suffix,mode..'.pa'))
  -- Native field-array probes also check ng normalization independently.
  if j.kind~='cartesian_a' or j.suffix=='Ax' then
    local bfield
    if j.kind=='scalar' then
      bfield=simion.experimental.bfield_of_scalar(inst[j.suffix],j.mu and pa[j.mu],j.anti)
    elseif cartesian then bfield=cartesian.bfield
    else
      bfield=simion.experimental.field_array{
        [j.kind=='axisymmetric_psi' and 'rA' or 'Az']=inst[j.suffix],antimirror=j.anti}.bfield
    end
    local probes=assert(io.open(filename(j.suffix,mode..'.b.csv'),'w'))
    for _,q in ipairs({{.37,.23,0},{.2,.6,0},{.8,.4,0}}) do
      local x,y,z=inst[j.suffix]:pa_to_wb_coords(q[1]*(pa[j.suffix].nx-1),q[2]*(pa[j.suffix].ny-1),q[3])
      local bx,by,bz=bfield(x,y,z)
      probes:write(string.format('%.17g,%.17g,%.17g,%.17g,%.17g,%.17g\n',x,y,z,bx,by,bz))
    end
    probes:close()
  end
end
manifest:close();times:close()
print('MAGNETIC_AUDIT_DONE,'..name..','..mode..',physical_scale='..strict_scale)
