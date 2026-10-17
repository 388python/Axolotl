import * as THREE from 'three'
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js'

import type { SeedMapStructurePiece } from './advanced'
import { seedMapLayoutPalette } from './structure-layout'

/** An on-demand spatial preview of engine bounding boxes, without invented blocks. */
export class SeedMapLayoutScene {
    private readonly renderer: THREE.WebGLRenderer
    private readonly scene = new THREE.Scene()
    private readonly camera = new THREE.PerspectiveCamera(50, 1, 0.1, 10000)
    private readonly controls: OrbitControls
    private readonly geometry = new THREE.BoxGeometry(1, 1, 1)
    private readonly material = new THREE.MeshBasicMaterial({
        transparent: true,
        opacity: 0.22,
        depthWrite: false,
    })
    private readonly outlineMaterial = new THREE.LineBasicMaterial()
    private readonly selectionMaterial = new THREE.LineBasicMaterial()
    private readonly selection: THREE.LineSegments
    private readonly mesh: THREE.InstancedMesh
    private readonly edges: THREE.LineSegments
    private readonly observer: ResizeObserver
    private readonly raycaster = new THREE.Raycaster()
    private readonly pointer = new THREE.Vector2()
    private readonly transforms: THREE.Matrix4[] = []
    private readonly center: THREE.Vector3
    private readonly radius: number
    private start?: { x: number; y: number }
    private frame?: number
    private disposed = false
    private readonly themeObserver: MutationObserver

    constructor(
        private readonly canvas: HTMLCanvasElement,
        private readonly pieces: SeedMapStructurePiece[],
        private readonly onSelect: (piece: SeedMapStructurePiece) => void,
    ) {
        this.renderer = new THREE.WebGLRenderer({
            canvas,
            antialias: true,
            alpha: true,
            preserveDrawingBuffer: true,
        })
        this.renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2))
        this.renderer.setClearColor(0, 0)
        const bounds = new THREE.Box3()
        for (const piece of pieces) {
            bounds.expandByPoint(new THREE.Vector3(piece.min!.x, piece.min!.y, piece.min!.z))
            bounds.expandByPoint(
                new THREE.Vector3(piece.max!.x + 1, piece.max!.y + 1, piece.max!.z + 1),
            )
        }
        this.center = bounds.getCenter(new THREE.Vector3())
        this.radius = Math.max(8, bounds.getSize(new THREE.Vector3()).length() / 2)
        this.camera.far = this.radius * 100
        this.mesh = new THREE.InstancedMesh(this.geometry, this.material, pieces.length)
        const edgeGeometry = new THREE.EdgesGeometry(this.geometry)
        const vertices = edgeGeometry.getAttribute('position')
        const lines: number[] = []
        const point = new THREE.Vector3()
        pieces.forEach((piece, index) => {
            const min = piece.min!,
                max = piece.max!
            const transform = new THREE.Matrix4().compose(
                new THREE.Vector3(
                    (min.x + max.x + 1) / 2,
                    (min.y + max.y + 1) / 2,
                    (min.z + max.z + 1) / 2,
                ).sub(this.center),
                new THREE.Quaternion(),
                new THREE.Vector3(max.x - min.x + 1, max.y - min.y + 1, max.z - min.z + 1),
            )
            this.transforms.push(transform)
            this.mesh.setMatrixAt(index, transform)
            for (let i = 0; i < vertices.count; i++) {
                point.fromBufferAttribute(vertices, i).applyMatrix4(transform)
                lines.push(point.x, point.y, point.z)
            }
        })
        edgeGeometry.dispose()
        this.edges = new THREE.LineSegments(new THREE.BufferGeometry(), this.outlineMaterial)
        this.edges.geometry.setAttribute('position', new THREE.Float32BufferAttribute(lines, 3))
        this.selection = new THREE.LineSegments(
            new THREE.EdgesGeometry(this.geometry),
            this.selectionMaterial,
        )
        this.selection.matrixAutoUpdate = false
        this.selection.visible = false
        this.scene.add(this.mesh, this.edges, this.selection)
        this.controls = new OrbitControls(this.camera, canvas)
        this.controls.screenSpacePanning = true
        this.controls.zoomToCursor = true
        this.controls.minDistance = 1
        this.controls.maxDistance = this.radius * 20
        this.controls.addEventListener('change', this.render)
        this.themeObserver = new MutationObserver(this.updatePalette)
        this.themeObserver.observe(document.documentElement, {
            attributes: true,
            attributeFilter: ['class', 'style'],
        })
        this.updatePalette()
        this.observer = new ResizeObserver(this.resize)
        this.observer.observe(canvas)
        canvas.addEventListener('pointerdown', this.pointerDown)
        canvas.addEventListener('pointerup', this.pointerUp)
        this.resize()
        this.fit()
    }

    private resize = () => {
        const rect = this.canvas.getBoundingClientRect()
        this.renderer.setSize(Math.max(1, rect.width), Math.max(1, rect.height), false)
        this.camera.aspect = Math.max(1, rect.width) / Math.max(1, rect.height)
        this.camera.updateProjectionMatrix()
        this.render()
    }

    private updatePalette = () => {
        const palette = seedMapLayoutPalette(this.canvas)
        this.outlineMaterial.color.set(palette.normal)
        this.selectionMaterial.color.set(palette.selected)
        this.pieces.forEach((_, index) =>
            this.mesh.setColorAt(index, new THREE.Color(palette.normal)),
        )
        if (this.mesh.instanceColor) this.mesh.instanceColor.needsUpdate = true
        this.render()
    }

    private render = () => {
        if (this.disposed || this.frame !== undefined) return
        this.frame = requestAnimationFrame(() => {
            this.frame = undefined
            this.renderer.render(this.scene, this.camera)
        })
    }

    fit() {
        const vertical = THREE.MathUtils.degToRad(this.camera.fov / 2)
        const horizontal = Math.atan(Math.tan(vertical) * this.camera.aspect)
        const distance = (this.radius / Math.sin(Math.min(vertical, horizontal))) * 1.15
        this.camera.position.set(distance * 0.6, distance * 0.55, distance * 0.6)
        this.controls.target.set(0, 0, 0)
        this.controls.update()
        this.render()
    }

    select(id?: number) {
        const index = this.pieces.findIndex((piece) => piece.id === id)
        this.selection.visible = index >= 0
        if (index >= 0) this.selection.matrix.copy(this.transforms[index])
        this.render()
    }

    private pointerDown = (event: PointerEvent) => {
        this.start = { x: event.clientX, y: event.clientY }
    }
    private pointerUp = (event: PointerEvent) => {
        if (
            event.button !== 0 ||
            !this.start ||
            Math.hypot(event.clientX - this.start.x, event.clientY - this.start.y) > 4
        )
            return
        this.start = undefined
        const rect = this.canvas.getBoundingClientRect()
        this.pointer.set(
            ((event.clientX - rect.left) / rect.width) * 2 - 1,
            (-(event.clientY - rect.top) / rect.height) * 2 + 1,
        )
        this.raycaster.setFromCamera(this.pointer, this.camera)
        const hit = this.raycaster.intersectObject(this.mesh)[0]
        if (hit?.instanceId !== undefined) this.onSelect(this.pieces[hit.instanceId])
    }

    dispose() {
        this.disposed = true
        if (this.frame !== undefined) cancelAnimationFrame(this.frame)
        this.observer.disconnect()
        this.themeObserver.disconnect()
        this.controls.dispose()
        this.canvas.removeEventListener('pointerdown', this.pointerDown)
        this.canvas.removeEventListener('pointerup', this.pointerUp)
        this.mesh.dispose()
        this.geometry.dispose()
        this.edges.geometry.dispose()
        this.material.dispose()
        this.outlineMaterial.dispose()
        this.selectionMaterial.dispose()
        this.selection.geometry.dispose()
        this.renderer.dispose()
    }
}
