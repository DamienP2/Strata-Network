import { Fragment, type ReactNode } from 'react';

/** Un texte traduit dont certains `{nom}` deviennent des éléments (ex. une version en gras). */
export function rich(text: string, nodes: Record<string, ReactNode>): ReactNode {
  const parts = text.split(/\{(\w+)\}/g);
  return parts.map((p, i) => (i % 2 ? <Fragment key={i}>{p in nodes ? nodes[p] : `{${p}}`}</Fragment> : p));
}
