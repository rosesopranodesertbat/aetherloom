import { handleRequest } from "./gateway";
import { MatchmakingShardObject } from "./matchmaking";
import { ProfileIslandObject } from "./profile-island";
import { consumeSettlements } from "./settlements";
import type { Env, QueuedSettlement } from "./types";

export { MatchmakingShardObject, ProfileIslandObject };

export default {
  fetch(request: Request, env: Env): Promise<Response> {
    return handleRequest(request, env);
  },

  queue(batch: MessageBatch<QueuedSettlement>, env: Env): Promise<void> {
    return consumeSettlements(batch, env);
  },
} satisfies ExportedHandler<Env, QueuedSettlement>;
