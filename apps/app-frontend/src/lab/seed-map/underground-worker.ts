import type { UndergroundMeshRequest } from './underground'
import { buildUndergroundMesh } from './underground-mesh'

const scope = self as DedicatedWorkerGlobalScope
scope.onmessage = (event: MessageEvent<UndergroundMeshRequest>) => {
    try {
        const mesh = buildUndergroundMesh(event.data)
        const buffers = Object.values(mesh.layers).flatMap((layer) =>
            Object.values(layer).map((data) => data.buffer),
        )
        scope.postMessage({ mesh }, buffers)
    } catch (error) {
        scope.postMessage({ error: error instanceof Error ? error.message : String(error) })
    }
}
