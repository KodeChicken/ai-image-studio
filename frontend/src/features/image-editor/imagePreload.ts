const imageCache = new Map<string, Promise<HTMLImageElement>>()

export function loadEditorImage(url: string): Promise<HTMLImageElement> {
  const cached = imageCache.get(url)
  if (cached) return cached

  const pending = new Promise<HTMLImageElement>((resolve, reject) => {
    const image = new Image()
    image.onload = () => resolve(image)
    image.onerror = () => reject(new Error('image load failed'))
    image.src = url
  })
  imageCache.set(url, pending)
  void pending.catch(() => {
    if (imageCache.get(url) === pending) imageCache.delete(url)
  })
  return pending
}

export function preloadEditorImage(url: string) {
  void loadEditorImage(url).catch(() => undefined)
}

export function evictEditorImage(url: string) {
  imageCache.delete(url)
}
