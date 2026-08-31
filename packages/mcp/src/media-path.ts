import path from "node:path";
import type { Project } from "@cut/timeline";

function isContained(root: string, candidate: string): boolean {
  const relative = path.relative(root, candidate);
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

export function resolveMediaSrc(dir: string, src: string, project?: Project): string {
  const resolvedDir = path.resolve(dir);
  const resolved = path.isAbsolute(src) ? path.resolve(src) : path.resolve(resolvedDir, src);
  if (isContained(resolvedDir, resolved)) return resolved;
  if (project !== undefined) {
    const matchesSource = project.sources.some((source) => {
      const sourcePath = path.isAbsolute(source.path)
        ? path.resolve(source.path)
        : path.resolve(resolvedDir, source.path);
      return sourcePath === resolved;
    });
    if (matchesSource) return resolved;
  }
  throw new Error(`media path is outside the project directory: ${src}`);
}

export function resolveImportPath(dir: string, inputPath: string): string {
  return path.isAbsolute(inputPath) ? path.resolve(inputPath) : path.resolve(dir, inputPath);
}
