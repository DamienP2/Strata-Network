import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import './styles.css';
import './styles-light.css';
import './theme';
import { App } from './App';
import { installNumberWheel } from './ui/fields';
import { installTooltips } from './ui/tooltips';
import { installEscape } from './ui/escape';

installNumberWheel();
installTooltips();
installEscape();

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
