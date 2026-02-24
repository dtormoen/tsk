---
description: Create an implementation plan
---

# Implementation Plan Request
Write a detailed document describing how to implement the following changes to the repository:

{{PROMPT}}

## Planning Best Practices
- Think carefully about how to achieve the stated goal
- Consider the goal of the plan. Is there a better way to achieve the goal which has not been considered yet?
- Carefully consider options to achieve the goal. Favor simplicity and long term scalability as the project grows
- Consider how the suggested changes should be tested and include this in the plan
- IMPORTANT: Don't write any code, only produce the requested document

## Document Format
The document you produce should have the following format:

```md
## Summary

A short summary of the requested changes.

## Options Considered

An overview of more than 1 options considered to accomplish the goal. This section should touch upon trade-offs and any other considerations for the different options presented

## Recommendation

Provide a recommendation for the best approach to implement the requested changes.

## Implementation Plan
This section should contain everything needed for someone or an agent to implement the plan. Start with a short, high level of the summary of the changes in the first paragraph to give an overview.

After the summary, write out concise, but comprehensive steps for implementing the change. For larger changes, the changes can broken down into phases, but only do this for very complex changes. Reference the specific files, classes, and functions that need to be added, modified, or deleted.

### Success Criteria
This section should include a checklist of criteria to successfully complete the change.

### Out of Scope
This section should contain any work items that should be saved for the future.
```

Save the completed doc to the docs folder with a short, descriptive title. Commit your doc following the Conventional Commits specification with a brief summary of the doc.
