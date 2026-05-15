# Operating Model

## Roles

Prime owns:

- task decomposition
- scope definition
- skill selection
- task envelope creation
- model selection
- model invocation
- patch review
- tests and checks
- acceptance or rejection
- project memory updates

The local model may return:

- analysis
- review findings
- file explanations
- test proposals
- unified diffs
- documentation drafts
- refusal with reason

## Recommended Flow

1. Read project config.
2. Select a narrow task.
3. Create or generate a task envelope.
4. Validate file scope.
5. Invoke the local model.
6. Treat output as advisory.
7. Apply patches only after review.
8. Run project checks.
9. Write a scorecard and retrospective when useful.

## First Pilot Tasks

Start with low-risk work:

- Review a selected git diff.
- Explain one module.
- Propose missing tests for one small file.
- Identify documentation drift.

Avoid early:

- auth changes
- database migrations
- dependency updates
- generated files
- large refactors
- security-sensitive architecture
