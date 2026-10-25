import { useEffect, useState } from "react";
import { useResolvedTheme } from "../../state/theme";
import { readPlotTheme, type PlotTheme } from "./chart";

/**
 * Plot colours from the CSS tokens. Re-read in an effect after a theme switch: App applies
 * data-theme in a layout effect, which has run by then.
 */
export function usePlotTheme(): PlotTheme {
  const resolved = useResolvedTheme();
  const [theme, setTheme] = useState(readPlotTheme);
  useEffect(() => setTheme(readPlotTheme()), [resolved]);
  return theme;
}
