import { cloneEditorDocument, type ImageEditorDocumentV1 } from './editorDocument'

export class EditorHistory {
  private past: ImageEditorDocumentV1[] = []
  private future: ImageEditorDocumentV1[] = []

  constructor(private readonly limit = 100) {}

  get canUndo(): boolean { return this.past.length > 0 }
  get canRedo(): boolean { return this.future.length > 0 }

  push(previous: ImageEditorDocumentV1): void {
    this.past.push(cloneEditorDocument(previous))
    if (this.past.length > this.limit) this.past.shift()
    this.future = []
  }

  undo(current: ImageEditorDocumentV1): ImageEditorDocumentV1 | null {
    const previous = this.past.pop()
    if (!previous) return null
    this.future.push(cloneEditorDocument(current))
    return cloneEditorDocument(previous)
  }

  redo(current: ImageEditorDocumentV1): ImageEditorDocumentV1 | null {
    const next = this.future.pop()
    if (!next) return null
    this.past.push(cloneEditorDocument(current))
    return cloneEditorDocument(next)
  }

  clear(): void {
    this.past = []
    this.future = []
  }
}
