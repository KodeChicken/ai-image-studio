const loadImageEditorView = () => import('@/features/image-editor/ImageEditorView.vue')

export function preloadImageEditor() {
  void loadImageEditorView().catch(() => undefined)
}

export { loadImageEditorView }
