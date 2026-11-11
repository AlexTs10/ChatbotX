// Runs test files in path order, so `tests/smoke/99-*` (the interception
// self-check) runs after every other file.

import { BaseSequencer, type TestSpecification } from "vitest/node"

export class PathOrderSequencer extends BaseSequencer {
  override async sort(files: TestSpecification[]): Promise<TestSpecification[]> {
    return [...files].sort((a, b) => a.moduleId.localeCompare(b.moduleId))
  }

  override async shard(files: TestSpecification[]): Promise<TestSpecification[]> {
    return files
  }
}
