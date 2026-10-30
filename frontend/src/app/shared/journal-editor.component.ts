import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  effect,
  inject,
  input,
  output,
  signal,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { QuillEditorComponent } from 'ngx-quill';

@Component({
  selector: 'app-journal-editor',
  imports: [FormsModule, QuillEditorComponent, MatButtonModule],
  template: `
    <div class="editor-shell" [class.compact]="compact()">
      <quill-editor
        [ngModel]="draft()"
        (ngModelChange)="onChange($event)"
        (onBlur)="saveNow()"
        [modules]="modules"
        [placeholder]="placeholder()"
        [readOnly]="readonly()"
        format="html"
        theme="snow"
      />
      <div class="editor-foot">
        <span class="state">
          @if (saving()) {
            Guardando…
          } @else if (dirty()) {
            Cambios sin guardar
          } @else {
            Guardado automáticamente
          }
        </span>
        <button
          mat-button
          color="primary"
          type="button"
          [disabled]="readonly() || saving() || !dirty()"
          (click)="saveNow()"
        >
          Guardar
        </button>
      </div>
    </div>
  `,
  styles: `
    .editor-shell {
      display: flex;
      flex-direction: column;
      gap: 8px;
    }
    .editor-foot {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 10px;
    }
    .state {
      color: var(--muted-2);
      font-size: 10.5px;
      letter-spacing: 0.02em;
    }
    .compact .ql-editor {
      min-height: 110px;
    }
  `,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class JournalEditorComponent {
  readonly value = input<string>('');
  readonly placeholder = input('Escribe tu análisis, contexto y conclusiones…');
  readonly readonly = input(false);
  readonly saving = input(false);
  readonly compact = input(false);
  readonly save = output<string>();

  protected readonly draft = signal('');
  protected readonly dirty = signal(false);

  protected readonly modules = {
    toolbar: [
      [{ header: [2, 3, false] }],
      ['bold', 'italic', 'underline', 'strike'],
      [{ list: 'ordered' }, { list: 'bullet' }],
      ['blockquote', 'code-block'],
      ['link'],
      ['clean'],
    ],
  };

  constructor() {
    effect(() => {
      const value = this.value();
      if (!this.dirty()) {
        this.draft.set(value ?? '');
      }
    });
    inject(DestroyRef).onDestroy(() => {
      if (this.dirty()) {
        this.save.emit(this.draft());
      }
    });
  }

  protected onChange(html: string): void {
    this.draft.set(html ?? '');
    this.dirty.set(true);
  }

  protected saveNow(): void {
    if (!this.dirty() || this.readonly()) return;
    this.dirty.set(false);
    this.save.emit(this.draft());
  }
}
