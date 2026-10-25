-- Adapt a copied one-instance IOB; never touch the installed example.
simion.wb:load('quad.iob')
assert(#simion.wb.instances == 1)
local inst = simion.wb.instances[1]
inst.x,inst.y,inst.z = 0,0,0
inst.ox,inst.oy,inst.oz = 0,0,0
inst.az,inst.el,inst.rt = 0,0,0
inst.scale = 1
simion.wb.bounds = {xl=0,yl=-18,zl=-18,xr=60,yr=18,zr=18}
inst.pa:fast_adjust{[1]=2.5,[2]=-2.5,[3]=0}
inst.pa:save('quad.pa0')
simion.wb:save('compute.iob')
simion.wb:save('readout.iob')
print('ORIGIN',inst:pa_to_wb_coords(0,0,0))
print('XNODE',inst:pa_to_wb_coords(1,0,0))
