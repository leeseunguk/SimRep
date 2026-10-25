-- Read-only magnetic field probes and adjusted PA/placement export.
-- Run from a dedicated Windows work directory.
simion.wb:load(arg[1])
local b = simion.wb.bounds
print(string.format('BOUNDS,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g',
  b.xl,b.yl,b.zl,b.xr,b.yr,b.zr))
for n=1,#simion.wb.instances do
  local inst = simion.wb.instances[n]
  local pa = inst.pa
  local x,y,z = inst:pa_to_wb_coords(0,0,0)
  local axes = {}
  for a=1,3 do
    local p = {0,0,0}; p[a]=1
    local xx,yy,zz = inst:pa_to_wb_coords(unpack(p))
    -- This transform includes anisotropic PA spacings, not just placement
    -- scale. Normalize each direction rather than treating spacing as shear.
    local length = math.sqrt((xx-x)^2+(yy-y)^2+(zz-z)^2)
    axes[#axes+1] = (xx-x)/length
    axes[#axes+1] = (yy-y)/length
    axes[#axes+1] = (zz-z)/length
  end
  local values = {x,y,z,unpack(axes)}
  values[#values+1] = inst.scale
  for i,v in ipairs(values) do values[i] = string.format('%.17g',v) end
  print('INSTANCE,'..n..','..pa.filename..','..table.concat(values,','))
  pa:save('instance'..n..'.pa')
  for i=1,19 do
    local xx = (pa.nx-1)*(0.17+0.66*i/20)
    local yy = (pa.ny-1)*(0.19+0.62*(i%7)/7)
    local zz = (pa.nz-1)*(0.23+0.54*(i%5)/5)
    if not pa:electrode(math.floor(xx+0.5),math.floor(yy+0.5),math.floor(zz+0.5)) then
      local px,py,pz = inst:pa_to_wb_coords(xx,yy,zz)
      local bx,by,bz = inst:field_wc(px,py,pz)
      print(string.format('FIELD,%d,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g',
        n,px,py,pz,bx,by,bz))
    end
  end
end

local MF = require 'simionx.MField'
local V = MF.vector
local fields = {
  LOOP=MF.hoop{center=V(0,0,0),normal=V(0,0,1),radius=30,current=100},
  HELMHOLTZ=MF.combined_field{
    MF.hoop{center=V(0,0,-15),normal=V(0,0,1),radius=30,current=100},
    MF.hoop{center=V(0,0,15),normal=V(0,0,1),radius=30,current=100}},
  SOLENOID=MF.solenoid_hoops{first=V(-50,0,0),last=V(50,0,0),radius=10,current=0.7958,nturns=100}
}
for name,field in pairs(fields) do
  for i=0,20 do
    local x,y,z = i*1.7-17, (i%3)*1.2, i*2.1-21
    local bx,by,bz = field(x,y,z)
    print(string.format('ANALYTIC,%s,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g',
      name,x,y,z,bx,by,bz))
  end
end
