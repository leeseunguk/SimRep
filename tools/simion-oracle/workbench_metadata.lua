-- Read-only IOB inspection through the documented SIMION Lua API.
simion.wb:load(arg[1])
local b = simion.wb.bounds
print(string.format('BOUNDS,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g',
  b.xl,b.yl,b.zl,b.xr,b.yr,b.zr))
for n = 1, #simion.wb.instances do
  local inst = simion.wb.instances[n]
  local x,y,z = inst:pa_to_wb_coords(0,0,0)
  print(string.format('INSTANCE,%d,%s,%.17g,%.17g,%.17g,%.17g',
    n,inst.pa.filename,x,y,z,inst.scale))
  for a = 1,3 do
    local v = {0,0,0}
    v[a] = 1
    local xx,yy,zz = inst:pa_to_wb_coords(unpack(v))
    print(string.format('AXIS,%d,%d,%.17g,%.17g,%.17g',
      n,a,(xx-x)/inst.scale,(yy-y)/inst.scale,(zz-z)/inst.scale))
  end
  if arg[2] then inst.pa:save(arg[2] .. n .. '.pa') end
end
