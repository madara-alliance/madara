# Orchestrator

## Project Quantum Leap: Expanding Our Horizons 🚀

```
[controller] -> [service] -> [client] -> [ resource / external service ]
```

## Architecture Overview

The orchestrator follows a clean architecture pattern with clear separation of concerns:

1. **Controller Layer**: Handles incoming requests and routes them to appropriate services
2. **Service Layer**: Implements business logic and orchestrates operations
3. **Client Layer**: Provides interfaces to external systems (DB, cache, network)
4. **Resource Layer**: Manages cloud provider resources and external services

## Directory Structure

```
crates/
├── orchestrator/            # Command Center
│   ├── src/
│   │   ├── error/           # All the Error Handling Units
│   │   ├── config/          # Application Parameters (like cli params)
│   │   ├── metadata/        # Metadata Management Units [Optional]
│   │   ├── resource/        # Resource Management Units (All Resources with Cloud provider)
│   │   │   ├── aws.rs       # AWS Resource Management
│   │   ├── core/            # Core Domain Logic and Abstractions
│   │   │   ├── client/      # Generic Client Interface Abstractions
│   │   │   │   ├── database.rs    # Database Client Interface
│   │   │   │   ├── queue.rs       # Queue Client Interface
│   │   │   │   ├── storage.rs     # Storage Client Interface
│   │   │   │   ├── notification.rs # Notification Client Interface
│   │   │   │   ├── event_bus.rs   # Event Bus Client Interface
│   │   │   │   ├── scheduler.rs   # Scheduler Client Interface
│   │   │   │   └── mod.rs
│   │   │   ├── madara/      # Madara-Specific Abstractions
│   │   │   │   ├── cron.rs        # Cron Client Interface
│   │   │   │   ├── job_queue.rs   # Job Queue Client Interface
│   │   │   │   ├── settlement.rs  # Settlement Client Interface
│   │   │   │   ├── da.rs          # Data Availability Client Interface
│   │   │   │   └── mod.rs
│   │   │   └── mod.rs
│   │   ├── client/          # Client Implementations
│   │   │   ├── db/          # Database Client Implementations
│   │   │   │   ├── mongodb.rs    # MongoDB Client Implementation
│   │   │   │   └── mod.rs
│   │   │   ├── storage/     # Storage Client Implementations
│   │   │   │   ├── s3.rs         # S3 Client Implementation
│   │   │   │   └── mod.rs
│   │   │   ├── queue/       # Queue Client Implementations
│   │   │   ├── cron/        # Cron Client Implementations
│   │   │   │   ├── event_bridge.rs # EventBridge Cron Implementation
│   │   │   │   └── mod.rs
│   │   │   ├── job_queue/   # Job Queue Client Implementations
│   │   │   │   ├── sqs.rs        # SQS Job Queue Implementation
│   │   │   │   └── mod.rs
│   │   │   ├── settlement/  # Settlement Client Implementations
│   │   │   │   ├── ethereum.rs   # Ethereum Settlement Implementation
│   │   │   │   ├── starknet.rs   # Starknet Settlement Implementation
│   │   │   │   └── mod.rs
│   │   │   ├── da/          # Data Availability Client Implementations
│   │   │   │   ├── ethereum.rs   # Ethereum DA Implementation
│   │   │   │   └── mod.rs
│   │   │   ├── cache/       # Cache Client Implementations
│   │   │   ├── network/     # Network Client Implementations
│   │   │   └── mod.rs
│   │   ├── controller/      # Request Handling and Routing
│   │   │   ├── route/       # API Routes and Handlers
│   │   │   ├── worker/      # Background Worker Processes
│   │   │   └── mod.rs
│   │   ├── service/         # Business Logic Layer
│   │   │   ├── job/         # Job Processing Services
│   │   │   ├── block/       # Block Processing Services
│   │   │   ├── proof/       # Proof Generation Services
│   │   │   └── mod.rs
│   │   ├── utils/           # Universal Toolkit Collection
│   │   ├── setup.rs         # Initial Setup for Resources
│   │   └── main.rs          # Prime Initialization Sequence
│   ├── Cargo.toml
│   └── README.md
```

## Client Abstraction Layer

The client abstraction layer is organized under `core/client/` and provides interfaces for all external system interactions:

### Database Client (`core/client/database.rs`)
- Defines the `DatabaseClient` trait for database operations
- Operations: connect, disconnect, insert, find, update, delete, count

### Queue Client (`core/client/queue.rs`)
- Defines the `QueueClient` trait for message queue operations
- Operations: connect, send, receive, delete, change visibility, purge

### Storage Client (`core/client/storage.rs`)
- Defines the `StorageClient` trait for object storage operations
- Operations: init, bucket management, object CRUD, presigned URLs

### Notification Client (`core/client/notification.rs`)
- Defines the `NotificationClient` trait for notification service operations
- Operations: init, topic management, subscription management, publish

### Event Bus Client (`core/client/event_bus.rs`)
- Defines the `EventBusClient` trait for event bus operations
- Operations: init, event bus management, event publishing, rule management

### Scheduler Client (`core/client/scheduler.rs`)
- Defines the `SchedulerClient` trait for scheduled task management
- Operations: init, schedule management, enable/disable schedules

## Madara-Specific Abstraction Layer

The Madara-specific abstraction layer is organized under `core/madara/` and provides interfaces for Madara-specific functionality:

### Cron Client (`core/madara/cron.rs`)
- Defines the `CronClient` trait for schedule management
- Provides functionality for creating and managing scheduled jobs
- Operations: create schedule, add target, enable/disable schedules
- Clean abstraction with proper separation from implementation details

### Job Queue Client (`core/madara/job_queue.rs`)
- Defines the `JobQueueClient` trait for job queue operations
- Manages job processing and verification queues
- Operations: send/receive messages, create queues, manage jobs
- Enhanced with job type mapping and specialized methods for job handling

### Settlement Client (`core/madara/settlement.rs`)
- Defines the `SettlementClient` trait for settlement layer operations
- Unified interface for updating state and registering proofs
- Operations: register proof, update state, verify transactions
- Supports multiple settlement layers (Ethereum, Starknet) with a clean abstraction

### Data Availability Client (`core/madara/da.rs`)
- Defines the `DaClient` trait for data availability operations
- Generalized interface for publishing and verifying data inclusion
- Operations: publish state diff, verify inclusion, validate data
- Provider-agnostic with proper separation of interface from implementation

## Client Implementation Layer

The client implementation layer is organized under `client/` and provides concrete implementations of the client abstractions:

### Database Clients (`client/db/`)
- `MongoDbClient`: MongoDB implementation of the `DatabaseClient` trait

### Storage Clients (`client/storage/`)
- `S3StorageClient`: AWS S3 implementation of the `StorageClient` trait

### Queue Clients (`client/queue/`)
- Implementations for SQS, Kafka, etc.

### Cache Clients (`client/cache/`)
- Implementations for Redis, in-memory cache, etc.

### Network Clients (`client/network/`)
- Implementations for HTTP, WebSocket, gRPC, etc.

## Flow of Control

1. **Controller** receives requests and routes them to appropriate services
2. **Service** implements business logic and orchestrates operations
3. **Service** uses **Client** interfaces to interact with external systems
4. **Client** implementations handle the details of external system interactions
5. **Resource** management handles cloud provider resources

## Dependency Direction

Dependencies flow inward:
- Controllers depend on Services
- Services depend on Client interfaces (abstractions)
- Client implementations depend on Client interfaces
- No inner layer depends on an outer layer

This ensures a clean separation of concerns and makes the system more maintainable and testable.




