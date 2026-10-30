import { gsap } from 'gsap';
import { Draggable } from 'gsap/Draggable';

gsap.registerPlugin(Draggable);

const query = matchMedia('(prefers-reduced-motion: reduce)');
let reduced = query.matches;
query.addEventListener('change', (e) => (reduced = e.matches));

/** True when the user has not asked the OS for reduced motion. */
export const animate = () => !reduced;

/** Duration helper for Svelte transitions: 0 when motion is reduced. */
export const ms = (value: number) => (reduced ? 0 : value);

export { gsap, Draggable };
