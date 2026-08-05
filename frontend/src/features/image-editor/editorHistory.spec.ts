import { describe, expect, it } from 'vitest'
import { createEditorDocument } from './editorDocument'
import { EditorHistory } from './editorHistory'

describe('editor history', () => {
  it('undoes and redoes exact document states', () => {
    const history = new EditorHistory()
    const first = createEditorDocument('asset', 1024, 1024)
    const second = structuredClone(first)
    second.canvas.width = 1920
    history.push(first)
    expect(history.undo(second)?.canvas.width).toBe(1024)
    expect(history.redo(first)?.canvas.width).toBe(1920)
  })
})
