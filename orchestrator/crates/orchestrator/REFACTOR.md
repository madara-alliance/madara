
## Thanks for your interest in contributing to the refactor

Thanks for you contribution to the refactor. and thanks for understanding the process.

### Motivation

The current orchestrator codebase has grown organically and now faces challenges with scalability, extensibility, and resilience. As we expand our blockchain L2 capabilities, we need a more robust architecture that can handle increased load, support multiple cloud providers, and recover gracefully from failures.

The existing code structure lacks clear separation of concerns, making it difficult to maintain and extend. Error handling is inconsistent, and there's tight coupling between components that should be independent.

This revamp will address these issues by implementing a clean, layered architecture with proper abstraction boundaries and well-defined interfaces between components.

## What we are objective to achieve

We want to refactor the orchestrator to make it more modular and easier to maintain.
and more scalable and resilient. we want to achieve the following objectives:

- Separate concerns: The orchestrator should be structured in a way that allows for better separation of concerns, making it easier to understand, maintain, and extend.
- Clear interfaces: The orchestrator should have clear interfaces between components, making it easier to integrate with other systems and services.
- Error handling: The orchestrator should have consistent error handling, making it easier to diagnose and fix issues.
- Scalability: The orchestrator should be able to handle increased load and support multiple cloud providers, making it more scalable and resilient.
- Extensibility: The orchestrator should be extensible, allowing for the addition of new features and capabilities without modifying the existing codebase.
- Resilience: The orchestrator should be resilient to failures, ensuring that it can recover gracefully and continue functioning even in the face of unexpected events.



The project structure will be:
```
crates/
├── orchestrator/            # Command Center
│   ├── src/
│   │   ├── error/           # Error Handling Units
│   │   ├── config/          # Application Parameters
│   │   ├── resource/        # Resource Management Units (setup, teardown)
│   │   ├── client/          # External Service Clients
│   │   ├── core/            # Domain Entities and Logic
│   │   ├── service/         # Business Layer
│   │   ├── controller/      # API and Worker Management
│   │   ├── utils/           # Universal Toolkit Collection
│   │   ├── setup.rs         # Initialization Setup
│   │   └── main.rs          # Application Entry Point
```

## PR Lineup

Note: all the Issue are under ```Project Orch``` [link](https://github.com/madara-alliance/madara/issues?q=state%3Aopen%20label%3A%22Project%20Orch%22)


Please review the following PRs in the order they appear:

1.

