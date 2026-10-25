-- Workbench readout only. Original per-PA legacy voltage/termination PRGs run.
simion.workbench_program()
sim_segment_global = 1
local next_trace = {}
function segment.initialize()
  next_trace[ion_number] = 0.25
  print(string.format('START,%d,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g',
    ion_number,ion_time_of_birth,ion_mass,ion_charge,
    ion_px_mm,ion_py_mm,ion_pz_mm,ion_vx_mm,ion_vy_mm,ion_vz_mm,ion_ke))
end
function segment.other_actions()
  if ion_time_of_flight >= (next_trace[ion_number] or math.huge) then
    next_trace[ion_number] = next_trace[ion_number] + 0.25
    print(string.format('TRACE,%d,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g',
      ion_number,ion_time_of_flight,ion_px_mm,ion_py_mm,ion_pz_mm,ion_vx_mm,ion_vy_mm,ion_vz_mm))
  end
end
function segment.terminate()
  print(string.format('FINAL,%d,%.17g,%.17g,%.17g,%.17g,%.17g',
    ion_number,ion_time_of_flight,ion_px_mm,ion_py_mm,ion_pz_mm,ion_ke))
end
