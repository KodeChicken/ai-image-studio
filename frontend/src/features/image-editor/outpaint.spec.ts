import { describe, expect, it } from 'vitest'
import type { ImageModel } from '@/types/api'
import { createEditorDocument } from './editorDocument'
import { chooseOutpaintSize, modelHasExplicitOutpaintSize, prepareOutpaint } from './outpaint'

function model(custom: boolean): ImageModel {
  return {
    id: 'model', providerId: 'provider', providerType: 'openai-compatible',
    modelKey: 'gpt-image-2', upstreamModelId: 'gpt-image-2', displayName: 'GPT Image 2',
    capabilities: {
      image_edit_capability: {
        supportedOutputSizes: custom ? 'custom' : ['1024x1024', '1536x1024', '1024x1536'],
      },
    },
    parameterSchema: {
      parameters: {
        size: custom ? {
          type: 'enum', allow_custom: true, options: ['1024x1024', '1536x1024', '1024x1536'],
          constraints: { edgeMultiple: 16, maxEdge: 3840, minPixels: 655360, maxPixels: 8294400, maxAspectRatio: 3 },
        } : { type: 'enum', options: ['1024x1024', '1536x1024', '1024x1536'] },
        n: { type: 'integer' }, output_format: { type: 'enum', options: ['png'] },
      },
    },
    availabilityStatus: 'verified', discoverySource: 'catalog', capabilitySource: 'catalog',
    lastDiscoveredAt: null, lastVerifiedAt: null, enabled: true,
  }
}

describe('outpaint preparation', () => {
  it('uses a valid exact custom size and maps the document without changing its source crop', () => {
    const document = createEditorDocument('asset', 1024, 1024)
    document.canvas.width = 1920
    document.canvas.height = 1088
    const prepared = prepareOutpaint(document, model(true))
    expect(prepared.size).toEqual({ width: 1920, height: 1088 })
    expect(prepared.parameters.size).toBe('1920x1088')
    expect(prepared.document.image.crop).toEqual(document.image.crop)
  })

  it('chooses the closest declared model canvas when the target is not directly supported', () => {
    expect(chooseOutpaintSize(model(false), { width: 1920, height: 1080 }))
      .toEqual({ width: 1536, height: 1024 })
  })

  it('requires a provider size parameter with an explicit pixel option or custom support', () => {
    expect(modelHasExplicitOutpaintSize(model(false))).toBe(true)
    const unsupported = model(false)
    unsupported.parameterSchema.parameters!.size = { type: 'enum', options: ['auto'] }
    unsupported.capabilities.image_edit_capability = { supportedOutputSizes: ['auto'] }
    expect(modelHasExplicitOutpaintSize(unsupported)).toBe(false)
  })
})
