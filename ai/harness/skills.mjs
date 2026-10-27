// Original SimRep implementation. Skills are trusted bundled documents, not workspace code.
import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'

export const RECIPES = [
  { id: 'einzel', keywords: /einzel|focus|lens|렌즈|초점/i, tools: [], title: 'Einzel focusing',
    steps: 'Use refine_geometry with all electrode assignments, CPU, gap_check error. Keep the PA0, numbered bases and cuts sibling together. Fly the same beam at several lens voltages using fast adjust; apply the detector aperture to plane hits. Minimize spot RMS at a fixed detector or locate the waist; report beam, focal metric, arrivals and three-grid u_num. Start from examples/data/einzel.gem and ions.ion only if the user selected that example.' },
  { id: 'esa', keywords: /\bESA\b|analy[sz]er|energy.channel|passband|FWHM|dE\/E|분석기|에너지.*채널/i, tools: ['analyzer_scan'], title: 'ESA energy channels',
    steps: 'Solve exact geometry once. Use analyzer_scan for each beam energy with absolute per-electrode channel voltages and an explicit detector aperture. Scan U around the estimated pass voltage; bracket both half-height crossings and use at least 8 points above half height. Centroid uses only the connected half-height peak; interpolate FWHM crossings. Fit k=sum(U_i E_i)/sum(E_i^2); state whether U is plate difference or one plate voltage. dE/E=FWHM_U/abs(U_i) assumes a linear analyzer. Converge centroid AND width on three grids and vary beam sample count.' },
  { id: 'quadrupole', keywords: /quadrupole|사중극|mass.filter/i, tools: [], title: 'Quadrupole transmission',
    steps: 'State rod geometry, mass/charge, injection emittance, RF frequency in inverse microseconds, RF phase and DC/RF convention. Refine rod unit bases once; fly with voltage_expr and param (time t is us). Count plane hits within the exit aperture, scan masses or DC/RF settings, and report transmission, mass bandwidth, duration/step limits. Tighten time integration tolerance and run a three-grid study; do not infer stability from one trajectory.' },
  { id: 'magnetic', keywords: /magnet|cyclotron|deflect|자기|자장|편향/i, tools: [], title: 'Magnetic deflection',
    steps: 'Use CPU for magnetic flight; B is gauss, positions mm, time us. Choose bfield, magnetic_pa or a coil source explicitly. State charge sign and SIMION azimuth convention (+90 degrees points -Z). Check deflection direction and analytic gyroradius/period where uniform B applies, compare Boris and tighter tolerance, then report detector hits and three-grid uncertainty for nonuniform fields.' },
  { id: 'cad', keywords: /\bCAD\b|STEP|IGES|gap|틈|간격|cad.measure|refine.geometry/i, tools: ['cad_measure', 'check_geometry'], title: 'CAD import and gaps',
    steps: 'Use check_geometry (or cad_measure) before solving STEP/IGES; it requires the cad build feature. Touching is a proximity classification, not proof of electrical contact. Assign every body exactly once, define electrical nets, then refine_geometry with gap_check error. Target h <= gap/10; fewer than 2.5 cells is a failure, not convergence. Keep origin, units, exact cuts and PA0 family consistent. GEM gap reports are certified brackets, not exact CAD distances.' },
]

export function selectRecipe(text = '', tool = '') {
  return RECIPES.find(recipe => recipe.keywords.test(text)) ?? RECIPES.find(recipe => recipe.tools.includes(tool))
}

export function skillCatalog() {
  return [{ id: 'ion-optics', title: 'SimRep ion optics workflow' }, ...RECIPES.map(({ id, title }) => ({ id, title }))]
}

export async function loadSkill(id) {
  if (id === 'ion-optics') {
    const file = fileURLToPath(new URL('../../.claude/skills/simrep-ion-optics/SKILL.md', import.meta.url))
    return { id, text: await readFile(file, 'utf8') }
  }
  const recipe = RECIPES.find(item => item.id === id)
  if (!recipe) throw new Error(`Unknown skill ${id}; choose ${skillCatalog().map(item => item.id).join(', ')}`)
  return { id, text: `# ${recipe.title}\n${recipe.steps}` }
}
