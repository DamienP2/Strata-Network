/**
 * Échap n'importe où « clique » le fond de la dernière modale ouverte (qui se
 * ferme alors comme sur un clic à côté), sauf si l'évènement est déjà traité.
 */
export function installEscape() {
  window.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape' || e.defaultPrevented) return;
    const backs = document.querySelectorAll<HTMLElement>('.modal-back');
    const last = backs[backs.length - 1];
    if (!last) return;
    e.preventDefault();
    last.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, cancelable: true }));
  });
}
