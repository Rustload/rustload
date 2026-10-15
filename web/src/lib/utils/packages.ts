import type { Package } from '$lib/api'
import { isActiveDownload } from './status'

export type PackageProgress = {
  downloaded: number
  total: number
  /** null when no size is known yet. */
  ratio: number | null
  speed: number
  /** Seconds until done at the current speed, null when idle or unknown. */
  eta: number | null
}

/** Aggregates live file values into package-level progress. */
export function packageProgress(pkg: Package): PackageProgress {
  let downloaded = 0
  let sizes = 0
  let speed = 0
  for (const file of pkg.files) {
    downloaded += file.downloaded
    sizes += file.size ?? 0
    speed += file.speed
  }
  const total = Math.max(sizes, pkg.total_size)
  const ratio = total > 0 ? Math.min(1, downloaded / total) : null
  const eta = speed > 0 && total > downloaded ? (total - downloaded) / speed : null
  return { downloaded, total, ratio, speed, eta }
}

/** Seconds spent downloading a finished package (first file start → finish). */
export function packageDuration(pkg: Package): number | null {
  if (pkg.finished_at === null) return null
  const starts = pkg.files.map((f) => f.started_at).filter((s): s is number => s !== null)
  const start = starts.length ? Math.min(...starts) : pkg.created_at
  return Math.max(0, pkg.finished_at - start)
}

/** Downloading here, at the debrid service, or being verified or extracted. */
export function isMovingPackage(pkg: Package): boolean {
  return (
    pkg.status === 'downloading' ||
    pkg.status === 'debrid' ||
    pkg.status === 'verifying' ||
    pkg.status === 'extracting' ||
    pkg.files.some((f) => isActiveDownload(f.status))
  )
}

/** Paused by the user (it may still be finishing remote or post-processing work). */
export function isPausedPackage(pkg: Package): boolean {
  return pkg.paused || pkg.status === 'paused'
}
