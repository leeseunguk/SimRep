-- Read-only evidence for artificial Dirichlet mirror planes.
simion.wb:load(arg[1])
local inst=simion.wb.instances[1]
local pa=inst.pa
local points={0,0.01,0.09,0.11,0.19,0.21,1,1.01,1.1,2}
if arg[2] == 'thin' then
  for x,y,z in pa:points() do
    pa:electrode(x,y,z, x==3)
    if x==3 then pa:potential(x,y,z,1000) end
  end
  points={2.1,2.49,2.51,2.9,3,3.01,3.49,3.51,3.9}
end
for _,x in ipairs(points) do
  local px,py,pz=inst:pa_to_wb_coords(x/pa.dx_mm,pa.ny/2,pa.nz/2)
  local status,nele,nmag=simion.wb:find_at(px,py,pz)
  print('POLE',x,pa:electrode(math.floor(x/pa.dx_mm+0.5),math.floor(pa.ny/2),math.floor(pa.nz/2)),
    status,nele,nmag)
end
