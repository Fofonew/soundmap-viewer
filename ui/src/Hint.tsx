// Infobulle « ⓘ » : au survol ou au focus clavier, affichée par-dessus tout
// (portail dans <body>, position fixe bornée à la fenêtre : jamais coupée par
// le défilement du panneau).
import { useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { t } from './i18n';

export function Hint({ children, label }: { children: ReactNode; label?: string }) {
  const icon = useRef<HTMLSpanElement>(null);
  const tip = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    if (!open || !icon.current || !tip.current) return;
    const a = icon.current.getBoundingClientRect();
    const t = tip.current.getBoundingClientRect();
    const m = 8;
    const left = Math.min(Math.max(m, a.left + a.width / 2 - t.width / 2), window.innerWidth - t.width - m);
    const below = a.bottom + 6;
    const top = below + t.height + m > window.innerHeight ? Math.max(m, a.top - t.height - 6) : below;
    setPos({ left, top });
  }, [open]);

  const show = () => setOpen(true);
  const hide = () => {
    setOpen(false);
    setPos(null);
  };

  return (
    <>
      <span
        ref={icon}
        className="hint"
        role="button"
        tabIndex={0}
        aria-label={label ?? t('common.help')}
        onMouseEnter={show}
        onMouseLeave={hide}
        onFocus={show}
        onBlur={hide}
        onClick={(e) => {
          // Dans un <label>, le clic ne doit pas activer le champ.
          e.preventDefault();
          e.stopPropagation();
          if (open) hide();
          else show();
        }}
        onKeyDown={(e) => e.key === 'Escape' && hide()}
      >
        ⓘ
      </span>
      {open &&
        createPortal(
          <div
            ref={tip}
            role="tooltip"
            className="tip"
            style={pos ? { left: pos.left, top: pos.top } : { left: -9999, top: 0, visibility: 'hidden' }}
          >
            {children}
          </div>,
          document.body,
        )}
    </>
  );
}
