import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import App from './App';
import './theme/tokens.css';
import './styles.css';
import './workspace-polish.css';
import './daily-driver.css';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
