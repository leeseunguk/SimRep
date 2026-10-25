-- Oracle instrumentation only; the native simulator never executes Lua.
simion.workbench_program()
sim_segment_global = 1

function segment.initialize_run()
  local f = assert(io.open('space_charge_case.txt', 'r'))
  local mode, amount, quality = f:read('*l'):match('(%S+)%s+(%S+)%s+(%S+)')
  f:close()
  sim_grouped = 1
  sim_relativity = 0
  sim_repulsion = mode
  sim_repulsion_amount = tonumber(amount)
  sim_trajectory_quality = tonumber(quality)
end

function segment.initialize()
  print(string.format('START,%d,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g',
    ion_number, ion_time_of_birth, ion_mass, ion_charge, ion_px_mm, ion_py_mm,
    ion_pz_mm, ion_vx_mm, ion_vy_mm, ion_vz_mm, ion_ke, ion_cwf))
end

function segment.terminate()
  print(string.format('FINAL,%d,%.17g,%.17g,%.17g,%.17g,%.17g',
    ion_number, ion_time_of_flight, ion_px_mm, ion_py_mm, ion_pz_mm, ion_ke))
end
