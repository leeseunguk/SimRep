-- Read-only inputs. Save only a separately named converged-example snapshot.
-- Run from the writable dielectric copy, with SIMION --nogui --noprompt lua.
simion.pas:close()
local pa = simion.pas:open('sphere_nonlinear_3dp.pa')
local base = simion.pas:open('sphere_nonlinear_3dp-dielectric.pa')
local epsilon = simion.pas:open('sphere_nonlinear_3dp-dielectric2.pa')
for repetition=1,3 do
  for x,y,z in pa:points() do
    pa:potential(x,y,z, x == 0 and 1e7 or 0)
  end
  for x,y,z in epsilon:points() do
    epsilon:potential(x,y,z,base:potential(x,y,z))
  end
  local start = os.clock()
  for iteration=1,20 do
    pa:refine{permittivity=epsilon, convergence=0.1, skipped_point=iteration<=5}
    for x,y,z in epsilon:points() do
      local xc,yc,zc = x+0.5,y+0.5,z+0.5
      local r = math.sqrt((xc-100)^2+yc^2+zc^2)
      local target = r>=20 and r<21 and 21 or r<20 and r>19 and 19 or r
      if target ~= r then
        local s=target/r
        xc,yc,zc=(xc-100)*s+100,yc*s,zc*s
      end
      local ex,ey,ez=pa:field_vc(xc,yc,zc)
      local e2=(ex^2+ey^2+ez^2)/(pa.dx_mm^2)*1e6
      local k0=base:potential(x,y,z)
      local k=math.max(k0*(1-(k0==6 and 8e-17 or 0)*e2),1)
      epsilon:potential(x,y,z,k)
    end
  end
  print(string.format('NONLINEAR_SIMION,%d,compute_s=%.9f,kcenter=%.12g,Ecenter=%.12g',
    repetition,os.clock()-start,epsilon:potential(100,0,0),
    (pa:potential(99,0,0)-pa:potential(101,0,0))/(2*pa.dx_mm)))
end
pa:save('sphere_nonlinear_3dp.simrep-nonlinear.pa')
