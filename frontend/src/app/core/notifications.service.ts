import { inject, Injectable } from '@angular/core';
import { MatSnackBar } from '@angular/material/snack-bar';

@Injectable({ providedIn: 'root' })
export class NotificationsService {
  private readonly snackbar = inject(MatSnackBar);

  success(message: string): void {
    this.snackbar.open(message, 'Cerrar', {
      duration: 3200,
      panelClass: ['snack-success'],
      horizontalPosition: 'right',
      verticalPosition: 'bottom',
    });
  }

  info(message: string): void {
    this.snackbar.open(message, 'Cerrar', {
      duration: 3200,
      panelClass: ['snack-info'],
      horizontalPosition: 'right',
      verticalPosition: 'bottom',
    });
  }

  error(message: unknown): void {
    this.snackbar.open(this.describe(message), 'Cerrar', {
      duration: 6000,
      panelClass: ['snack-error'],
      horizontalPosition: 'right',
      verticalPosition: 'bottom',
    });
  }

  private describe(error: unknown): string {
    if (typeof error === 'string') return error;
    if (error instanceof Error) return error.message;
    return 'Ha ocurrido un error inesperado';
  }
}
