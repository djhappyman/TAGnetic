import './fonts.css';
import './shadcn.css';
import './style.css';
import './components/selects.css';
import './surfaces.css';
import './palettes.css';
import './js/interface-fonts.js';
import { mount } from 'svelte';
import AudioSplitToolHost from './components/AudioSplitToolHost.svelte';
import { initTooltips } from './js/tooltip.js';
import { toolModes } from './js/audio-tools-window';

const target = document.getElementById('audio-tool')!;
const kind = toolModes.find(mode => mode === new URLSearchParams(location.search).get('tool'));
if (kind) {
  const root = document.documentElement;
  try {
    const theme = localStorage.getItem('tagrex.theme');
    const palette = localStorage.getItem('tagrex.palette');
    if (theme === 'light' || theme === 'dark') root.dataset.theme = theme;
    if (palette) root.dataset.palette = palette;
  } catch { /* The main window sends its resolved appearance after connecting. */ }
  mount(AudioSplitToolHost, { target, props: { kind } });
  initTooltips();
} else {
  target.textContent = 'Open an audio tool from TAGnetic.';
}
