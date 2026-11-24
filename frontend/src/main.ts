import { bootstrapApplication } from '@angular/platform-browser';
import { appConfig } from './app/app.config';
import { App } from './app/app';
import { applyTheme, readStoredTheme } from './app/core/theme.service';

// Aplica el tema guardado antes de arrancar para evitar un parpadeo.
applyTheme(readStoredTheme());

bootstrapApplication(App, appConfig)
  .catch((err) => console.error(err));
