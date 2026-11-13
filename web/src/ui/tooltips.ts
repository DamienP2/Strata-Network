/**
 * Infobulles dessinées : tout élément qui a un `title` montre une carte
 * `.tip` après 380 ms de survol, au lieu de l'infobulle du navigateur.
 */
const DELAY = 380;
const MARGIN = 8;

export function installTooltips() {
  if (document.getElementById('tip')) return;
  const tip = document.createElement('div');
  tip.id = 'tip';
  tip.className = 'tip';
  tip.setAttribute('role', 'tooltip');
  document.body.appendChild(tip);

  let target: HTMLElement | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let x = 0;
  let y = 0;

  const place = () => {
    const w = tip.offsetWidth;
    const h = tip.offsetHeight;
    let left = x + 14;
    let top = y + 18;
    if (left + w > innerWidth - MARGIN) left = Math.max(MARGIN, x - w - 8);
    if (top + h > innerHeight - MARGIN) top = Math.max(MARGIN, y - h - 10);
    tip.style.transform = `translate(${Math.round(left)}px, ${Math.round(top)}px)`;
  };
  const hide = () => {
    clearTimeout(timer);
    tip.classList.remove('on');
    target = null;
  };
  /** Le title devient data-tip : le navigateur n'affiche plus le sien. */
  const adopt = (el: HTMLElement) => {
    const title = el.getAttribute('title');
    if (title !== null) {
      el.removeAttribute('title');
      if (title) el.dataset.tip = title;
      else delete el.dataset.tip;
      if (title && !el.textContent?.trim() && !el.hasAttribute('aria-label')) el.setAttribute('aria-label', title);
    }
  };
  const find = (n: EventTarget | null): HTMLElement | null => {
    let el = n instanceof Element ? (n as HTMLElement) : null;
    while (el && el !== document.body) {
      if (el.hasAttribute('title')) adopt(el);
      if (el.dataset?.tip) return el;
      el = el.parentElement;
    }
    return null;
  };

  document.addEventListener('pointerover', (e) => {
    if (e.pointerType === 'touch') return;
    const el = find(e.target);
    if (el === target) return;
    hide();
    if (!el) return;
    target = el;
    x = e.clientX;
    y = e.clientY;
    timer = setTimeout(() => {
      if (target !== el || !el.isConnected || !el.dataset.tip) return;
      tip.textContent = el.dataset.tip;
      place();
      tip.classList.add('on');
    }, DELAY);
  });
  document.addEventListener('pointermove', (e) => {
    if (!target) return;
    x = e.clientX;
    y = e.clientY;
    if (tip.classList.contains('on')) place();
  });
  document.addEventListener('pointerout', (e) => {
    if (target && !target.contains(e.relatedTarget as Node | null)) hide();
  });
  document.addEventListener('pointerdown', hide, true);
  document.addEventListener('wheel', hide, { capture: true, passive: true });
  document.addEventListener('keydown', hide, true);
  window.addEventListener('blur', hide);
}
