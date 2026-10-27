import { ChangeDetectionStrategy, Component, inject, input, output, signal } from '@angular/core';
import { convertFileSrc } from '@tauri-apps/api/core';
import { MatButtonModule } from '@angular/material/button';
import { MatDialog } from '@angular/material/dialog';
import { MatIconModule } from '@angular/material/icon';
import { MatTooltipModule } from '@angular/material/tooltip';
import { firstValueFrom } from 'rxjs';

import { ApiService } from '../core/api.service';
import { NotificationsService } from '../core/notifications.service';
import type { ImageInfo } from '../core/models';
import {
  ConfirmDialogComponent,
  type ConfirmDialogResult,
} from './confirm-dialog.component';

@Component({
  selector: 'app-image-gallery',
  imports: [MatButtonModule, MatIconModule, MatTooltipModule],
  template: `
    <div class="gallery" [class.compact]="compact()">
      @for (image of images(); track image.id) {
        <figure class="shot" (click)="preview.set(image)">
          <img [src]="source(image)" [alt]="image.fileName" loading="lazy" />
          @if (editable()) {
            <button
              mat-icon-button
              class="remove"
              type="button"
              matTooltip="Eliminar imagen"
              (click)="remove(image, $event)"
            >
              <span class="material-symbols-outlined">close</span>
            </button>
          }
        </figure>
      }

      @if (editable()) {
        <button class="add-shot" type="button" (click)="picker.click()" [disabled]="uploading()">
          <span class="material-symbols-outlined">{{ uploading() ? 'hourglass_top' : 'add_photo_alternate' }}</span>
          <span>{{ uploading() ? 'Subiendo…' : 'Añadir captura' }}</span>
        </button>
      }

      @if (!images().length && !editable()) {
        <p class="muted text-xs">Sin capturas.</p>
      }
    </div>

    <input
      #picker
      type="file"
      hidden
      multiple
      accept="image/png,image/jpeg,image/webp,image/gif"
      (change)="upload($event)"
    />

    @if (preview(); as image) {
      <div class="lightbox" (click)="preview.set(null)">
        <button mat-icon-button class="close" type="button" (click)="preview.set(null)">
          <span class="material-symbols-outlined">close</span>
        </button>
        <img [src]="source(image)" [alt]="image.fileName" (click)="$event.stopPropagation()" />
        <div class="caption">{{ image.fileName }}</div>
      </div>
    }
  `,
  styles: `
    .gallery {
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(112px, 1fr));
      gap: 10px;
    }
    .gallery.compact {
      grid-template-columns: repeat(auto-fill, minmax(88px, 1fr));
    }
    .shot {
      position: relative;
      aspect-ratio: 4 / 3;
      margin: 0;
      border: 1px solid var(--border-strong);
      border-radius: 10px;
      overflow: hidden;
      cursor: zoom-in;
      background: var(--surface-2);

      img {
        width: 100%;
        height: 100%;
        object-fit: cover;
        display: block;
      }

      &:hover .remove {
        opacity: 1;
      }
    }
    .remove {
      position: absolute;
      top: 4px;
      right: 4px;
      opacity: 0;
      background: rgba(9, 13, 20, 0.8);
      color: var(--neg);
      transition: opacity 140ms ease;
      transform: scale(0.82);
    }
    .add-shot {
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      gap: 6px;
      aspect-ratio: 4 / 3;
      border: 1px dashed var(--border-strong);
      border-radius: 10px;
      background: transparent;
      color: var(--muted);
      cursor: pointer;
      font-size: 10.5px;

      &:hover {
        border-color: var(--accent);
        color: var(--accent);
      }

      .material-symbols-outlined {
        font-size: 24px;
      }
    }
    .lightbox {
      position: fixed;
      inset: 0;
      z-index: 1000;
      display: grid;
      place-items: center;
      padding: 40px;
      background: rgba(4, 7, 12, 0.9);
      backdrop-filter: blur(6px);

      img {
        max-width: min(1200px, 90vw);
        max-height: 82vh;
        border-radius: 12px;
        box-shadow: var(--shadow);
      }

      .close {
        position: absolute;
        top: 18px;
        right: 18px;
        color: white;
      }

      .caption {
        position: absolute;
        bottom: 20px;
        color: var(--muted);
        font-size: 11px;
      }
    }
  `,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ImageGalleryComponent {
  readonly ownerType = input.required<'day_journal' | 'day_asset' | 'trade'>();
  readonly ownerId = input.required<string>();
  readonly images = input<ImageInfo[]>([]);
  readonly editable = input(true);
  readonly compact = input(false);
  readonly changed = output<void>();

  private readonly api = inject(ApiService);
  private readonly dialog = inject(MatDialog);
  private readonly notifications = inject(NotificationsService);

  protected readonly uploading = signal(false);
  protected readonly preview = signal<ImageInfo | null>(null);

  protected source(image: ImageInfo): string {
    const path = image.thumbUrl || image.path;
    if (!path) return '';
    try {
      return convertFileSrc(path);
    } catch {
      return path;
    }
  }

  protected async upload(event: Event): Promise<void> {
    const input = event.target as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    input.value = '';
    if (!files.length) return;
    this.uploading.set(true);
    let uploaded = 0;
    try {
      for (const file of files) {
        if (file.size > 20 * 1024 * 1024) {
          this.notifications.error(`${file.name} supera los 20 MB`);
          continue;
        }
        const dataUrl = await this.readFile(file);
        await this.api.saveImage({
          ownerType: this.ownerType(),
          ownerId: this.ownerId(),
          fileName: file.name,
          caption: '',
          dataUrl,
        });
        uploaded += 1;
      }
      if (uploaded) {
        this.notifications.success(
          uploaded === 1 ? 'Captura añadida' : `${uploaded} capturas añadidas`,
        );
        this.changed.emit();
      }
    } catch (error) {
      this.notifications.error(error);
    } finally {
      this.uploading.set(false);
    }
  }

  protected async remove(image: ImageInfo, event: Event): Promise<void> {
    event.stopPropagation();
    const ref = this.dialog.open<ConfirmDialogComponent, unknown, ConfirmDialogResult>(
      ConfirmDialogComponent,
      {
        data: {
          title: 'Eliminar captura',
          message: 'La imagen se borrará del disco y del diario. Esta acción no se puede deshacer.',
          confirmText: 'Eliminar',
          danger: true,
          icon: 'delete',
        },
        width: '440px',
      },
    );
    const result = await firstValueFrom(ref.afterClosed());
    if (!result?.confirmed) return;
    try {
      await this.api.deleteImage(image.id);
      if (this.preview()?.id === image.id) this.preview.set(null);
      this.changed.emit();
    } catch (error) {
      this.notifications.error(error);
    }
  }

  private readFile(file: File): Promise<string> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(reader.result as string);
      reader.onerror = () => reject(new Error(`No se pudo leer ${file.name}`));
      reader.readAsDataURL(file);
    });
  }
}
