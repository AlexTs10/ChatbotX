import { usePipelineDetail } from "@/features/pipelines/lib/use-pipeline-detail"
import { useWorkspaceId } from "@/hooks/routing"

const NO_FIELDS: { id: string; name: string; required: boolean }[] = []

/** Custom fields of one pipeline, for the field pickers of the Pipelines step. */
export function usePipelineFields(pipelineId: string) {
  const workspaceId = useWorkspaceId()
  const query = usePipelineDetail(workspaceId ?? "", pipelineId, {
    enabled: Boolean(workspaceId) && Boolean(pipelineId),
  })
  // The SPA oRPC client is untyped (lib/orpc/orpc.ts): name the type.
  const data = query.data as { fields?: typeof NO_FIELDS } | undefined
  return { fields: data?.fields ?? NO_FIELDS }
}
