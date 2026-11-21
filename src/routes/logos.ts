import type { SvgData } from '@fuzdev/fuz_ui/svg.ts';

const color = '#2e9e6f';

/**
 * Cord's logo: two socket faces leaning apart, a plug reaching toward each,
 * and the one cord that joins the plugs, looped to one side. The faces are
 * the people and the cord is what they share.
 *
 * `static/logo.svg` is the same drawing as a file, and `static/favicon.png`
 * is rendered from it; change all three together.
 */
export const logo_cord = {
	label:
		'two teal socket faces leaning apart, a plug reaching toward each, joined by one looped cord',
	fill: color,
	paths: [
		{
			d: 'M32,9 H68 A15,15 0 0 1 83,24 V76 A15,15 0 0 1 68,91 H32 A15,15 0 0 1 17,76 V24 A15,15 0 0 1 32,9 Z M37,29 H40.5 A2,2 0 0 1 42.5,31 V48 A2,2 0 0 1 40.5,50 H37 A2,2 0 0 1 35,48 V31 A2,2 0 0 1 37,29 Z M59.5,29 H63 A2,2 0 0 1 65,31 V48 A2,2 0 0 1 63,50 H59.5 A2,2 0 0 1 57.5,48 V31 A2,2 0 0 1 59.5,29 Z M43.8,67 A6.2,6.2 0 1 0 56.2,67 A6.2,6.2 0 1 0 43.8,67 Z',
			'fill-rule': 'evenodd',
			transform: 'translate(15 29) rotate(-13) scale(0.38) translate(-50 -50)'
		},
		{
			d: 'M32,9 H68 A15,15 0 0 1 83,24 V76 A15,15 0 0 1 68,91 H32 A15,15 0 0 1 17,76 V24 A15,15 0 0 1 32,9 Z M37,29 H40.5 A2,2 0 0 1 42.5,31 V48 A2,2 0 0 1 40.5,50 H37 A2,2 0 0 1 35,48 V31 A2,2 0 0 1 37,29 Z M59.5,29 H63 A2,2 0 0 1 65,31 V48 A2,2 0 0 1 63,50 H59.5 A2,2 0 0 1 57.5,48 V31 A2,2 0 0 1 59.5,29 Z M43.8,67 A6.2,6.2 0 1 0 56.2,67 A6.2,6.2 0 1 0 43.8,67 Z',
			'fill-rule': 'evenodd',
			transform: 'translate(85 25) rotate(9) scale(0.38) translate(-50 -50)'
		},
		{
			d: 'M-9,-17 H-7 A1.5,1.5 0 0 1 -5.5,-15.5 V1.5 A1.5,1.5 0 0 1 -7,3 H-9 A1.5,1.5 0 0 1 -10.5,1.5 V-15.5 A1.5,1.5 0 0 1 -9,-17 Z',
			transform: 'translate(33.5 52) rotate(-50) scale(0.4)'
		},
		{
			d: 'M7,-17 H9 A1.5,1.5 0 0 1 10.5,-15.5 V1.5 A1.5,1.5 0 0 1 9,3 H7 A1.5,1.5 0 0 1 5.5,1.5 V-15.5 A1.5,1.5 0 0 1 7,-17 Z',
			transform: 'translate(33.5 52) rotate(-50) scale(0.4)'
		},
		{
			d: 'M-10,0 H10 A7,7 0 0 1 17,7 V15 A7,7 0 0 1 10,22 H-10 A7,7 0 0 1 -17,15 V7 A7,7 0 0 1 -10,0 Z',
			transform: 'translate(33.5 52) rotate(-50) scale(0.4)'
		},
		{
			d: 'M-9,20 L9,20 L5,31 L-5,31 Z',
			transform: 'translate(33.5 52) rotate(-50) scale(0.4)'
		},
		{
			d: 'M-9,-17 H-7 A1.5,1.5 0 0 1 -5.5,-15.5 V1.5 A1.5,1.5 0 0 1 -7,3 H-9 A1.5,1.5 0 0 1 -10.5,1.5 V-15.5 A1.5,1.5 0 0 1 -9,-17 Z',
			transform: 'translate(67 48) rotate(30) scale(0.4)'
		},
		{
			d: 'M7,-17 H9 A1.5,1.5 0 0 1 10.5,-15.5 V1.5 A1.5,1.5 0 0 1 9,3 H7 A1.5,1.5 0 0 1 5.5,1.5 V-15.5 A1.5,1.5 0 0 1 7,-17 Z',
			transform: 'translate(67 48) rotate(30) scale(0.4)'
		},
		{
			d: 'M-10,0 H10 A7,7 0 0 1 17,7 V15 A7,7 0 0 1 10,22 H-10 A7,7 0 0 1 -17,15 V7 A7,7 0 0 1 -10,0 Z',
			transform: 'translate(67 48) rotate(30) scale(0.4)'
		},
		{
			d: 'M-9,20 L9,20 L5,31 L-5,31 Z',
			transform: 'translate(67 48) rotate(30) scale(0.4)'
		},
		{
			d: 'M40.2,57.7 C40.7,58 41.8,58.9 43,60 C44.2,61 44.4,61.8 47.6,63.8 C50.8,65.8 57.6,69.1 62,72 C66.4,74.9 72.3,77.6 74,81 C75.7,84.4 74.7,90.1 72,92.5 C69.3,94.9 61.8,96.4 58,95.5 C54.2,94.6 50.1,90.3 49,87 C47.9,83.7 50,79.3 51.5,75.5 C53,71.7 56.3,66.7 57.8,63.9 C59.4,61.1 60,60.1 60.8,58.7 C61.6,57.4 62.3,56.1 62.6,55.6',
			fill: 'none',
			stroke: color,
			'stroke-width': 4.2,
			'stroke-linecap': 'round',
			'stroke-linejoin': 'round'
		}
	]
} satisfies SvgData;
