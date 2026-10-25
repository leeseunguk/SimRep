simion.workbench_program()
sim_segment_global = 1
function segment.terminate()
  print(string.format('FINAL,%d,%.17g,%.17g,%.17g,%.17g,%.17g',
    ion_number,ion_time_of_flight,ion_px_mm,ion_py_mm,ion_pz_mm,ion_ke))
end
