// line 1
// line 2
// line 3
// line 4
// line 5
// line 6
// line 7
// line 8
// line 9
// line 10
// line 11
// line 12
// line 13
// line 14
// line 15
// line 16
// line 17
// line 18
// line 19
// line 20
// line 21
// line 22
// line 23
// line 24
// line 25
// line 26
// line 27
// line 28
// line 29
// line 30
// line 31
// line 32
// line 33
// line 34
// line 35
// line 36
// line 37
// line 38
// line 39
// line 40
// line 41
// line 42
// line 43
// line 44
// line 45
// line 46
// line 47
// line 48
// line 49
// line 50
  prompt: procedure
    .input(z.object({ sessionId: z.string(), text: z.string() }))
    .handler(async ({ input, context }) => {
      log.info({ event: "prompt", sessionId: input.sessionId, textLength: input.text.length });
      const session = context.registry.open(input.sessionId);
      // Resolves when the agent run ends (after agent_end); the reply
      // streams over the events subscription, not this call.
      await session.prompt(input.text);
      return;
    }),
// end of stub
