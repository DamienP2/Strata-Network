import { AudioLines, FileText, LayoutGrid, Radio, Video, type LucideIcon } from 'lucide-react';
import type { Tone } from '../../net/mcast';

const TONE_ICON: Record<Tone, LucideIcon> = { v: Video, a: AudioLines, d: FileText, ts: LayoutGrid, o: Radio };

/** L'icône d'un type de flux. */
export const toneIcon = (k: Tone): LucideIcon => TONE_ICON[k];
