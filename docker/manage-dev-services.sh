#!/bin/bash

# Script to manage LocalStack and MongoDB services for orchestrator development

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Script directory
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPT_DIR"

# Function to print colored output
print_status() {
    echo -e "${GREEN}$1${NC}"
}

print_error() {
    echo -e "${RED}$1${NC}"
}

print_warning() {
    echo -e "${YELLOW}$1${NC}"
}

print_info() {
    echo -e "${BLUE}$1${NC}"
}

# Function to check if service is running
check_service() {
    local service=$1
    if docker ps | grep -q $service; then
        return 0
    else
        return 1
    fi
}

# Function to wait for service to be healthy
wait_for_health() {
    local service=$1
    local max_attempts=30
    local attempt=0

    print_info "Waiting for $service to be healthy..."

    while [ $attempt -lt $max_attempts ]; do
        if docker inspect --format='{{.State.Health.Status}}' $service 2>/dev/null | grep -q "healthy"; then
            print_status "✓ $service is healthy"
            return 0
        fi
        sleep 2
        attempt=$((attempt + 1))
        echo -n "."
    done

    print_error "✗ $service failed to become healthy"
    return 1
}

# Command functions
start_services() {
    print_status "🚀 Starting LocalStack and MongoDB services..."

    # Pull images first
    print_info "Pulling Docker images..."
    docker pull localstack/localstack@sha256:763947722c6c8d33d5fbf7e8d52b4bddec5be35274a0998fdc6176d733375314
    docker pull mongo:latest

    # Start services
    docker compose -f docker-compose.dev.yml up -d

    # Wait for services to be healthy
    wait_for_health "mongodb"
    wait_for_health "localstack"

    # Run initialization
    print_info "Running LocalStack initialization..."
    sleep 5  # Give LocalStack a moment to fully start
    docker exec localstack sh /docker-entrypoint-initaws.d/01-create-resources.sh

    print_status "✅ All services started successfully!"
    echo ""
    show_status
}

stop_services() {
    print_warning "Stopping LocalStack and MongoDB services..."
    docker compose -f docker-compose.dev.yml down
    print_status "✓ Services stopped"
}

restart_services() {
    stop_services
    sleep 2
    start_services
}

show_status() {
    print_status "📊 Service Status:"
    echo "=================="

    if check_service "localstack"; then
        print_status "✓ LocalStack is running"
        echo "  - Gateway: http://localhost:4566"
        echo "  - Health: http://localhost:4566/_localstack/health"
    else
        print_error "✗ LocalStack is not running"
    fi

    if check_service "mongodb"; then
        print_status "✓ MongoDB is running"
        echo "  - Connection: mongodb://admin:admin123@localhost:27017"
        echo "  - Database: orchestrator"
    else
        print_error "✗ MongoDB is not running"
    fi

    if check_service "mongo-express"; then
        print_status "✓ Mongo Express is running"
        echo "  - URL: http://localhost:8081 (admin/pass)"
    fi

    echo ""
}

show_logs() {
    local service=${1:-all}

    if [ "$service" = "all" ]; then
        docker compose -f docker-compose.dev.yml logs -f
    else
        docker compose -f docker-compose.dev.yml logs -f $service
    fi
}

test_connections() {
    print_status "🔍 Testing service connections..."
    echo ""

    # Test LocalStack
    print_info "Testing LocalStack..."
    if curl -s http://localhost:4566/_localstack/health | grep -q "\"services\":"; then
        print_status "✓ LocalStack is accessible"

        # List S3 buckets
        aws --endpoint-url=http://localhost:4566 s3 ls 2>/dev/null && \
            print_status "✓ S3 buckets accessible" || \
            print_warning "⚠ S3 buckets not accessible"

        # List SQS queues
        aws --endpoint-url=http://localhost:4566 sqs list-queues 2>/dev/null && \
            print_status "✓ SQS queues accessible" || \
            print_warning "⚠ SQS queues not accessible"
    else
        print_error "✗ LocalStack is not accessible"
    fi

    echo ""

    # Test MongoDB
    print_info "Testing MongoDB..."
    if docker exec mongodb mongosh --quiet --eval "db.adminCommand('ping')" 2>/dev/null | grep -q "1"; then
        print_status "✓ MongoDB is accessible"

        # Check orchestrator database
        if docker exec mongodb mongosh orchestrator --quiet --eval "db.getCollectionNames()" 2>/dev/null | grep -q "jobs"; then
            print_status "✓ Orchestrator database initialized"
        else
            print_warning "⚠ Orchestrator database not initialized"
        fi
    else
        print_error "✗ MongoDB is not accessible"
    fi

    echo ""
}

clean_data() {
    print_warning "⚠️  This will delete all data in LocalStack and MongoDB!"
    read -p "Are you sure? (y/N) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        docker compose -f docker-compose.dev.yml down -v
        rm -rf ${TMPDIR:-/tmp}/localstack
        print_status "✓ All data cleaned"
    else
        print_info "Cancelled"
    fi
}

# Environment setup
setup_env() {
    print_status "Setting up environment variables..."

    cat > ../.env.dev << EOF
# AWS LocalStack Configuration
AWS_ACCESS_KEY_ID=test
AWS_SECRET_ACCESS_KEY=test
AWS_DEFAULT_REGION=us-east-1
AWS_ENDPOINT_URL=http://localhost:4566

# MongoDB Configuration
MONGODB_URI=mongodb://orchestrator_user:orchestrator_pass@localhost:27017/orchestrator
MONGODB_ROOT_URI=mongodb://admin:admin123@localhost:27017

# S3 Buckets
S3_BUCKET_PROOFS=madara-orchestrator-proofs
S3_BUCKET_ARTIFACTS=madara-orchestrator-artifacts
S3_BUCKET_SNOS_OUTPUT=madara-orchestrator-snos-output

# SQS Queues
SQS_QUEUE_PROCESSING=http://localhost:4566/000000000000/orchestrator-job-processing-queue
SQS_QUEUE_VERIFICATION=http://localhost:4566/000000000000/orchestrator-job-verification-queue

# SNS Topics
SNS_TOPIC_JOB_STATUS=arn:aws:sns:us-east-1:000000000000:orchestrator-job-status
SNS_TOPIC_ALERTS=arn:aws:sns:us-east-1:000000000000:orchestrator-alerts
EOF

    print_status "✓ Environment file created at ../.env.dev"
    echo ""
    echo "To use these variables, run:"
    echo "  export \$(cat .env.dev | xargs)"
}

# Main menu
show_help() {
    echo "LocalStack & MongoDB Development Services Manager"
    echo "================================================"
    echo ""
    echo "Usage: $0 [command]"
    echo ""
    echo "Commands:"
    echo "  start      - Start LocalStack and MongoDB"
    echo "  stop       - Stop all services"
    echo "  restart    - Restart all services"
    echo "  status     - Show service status"
    echo "  logs       - Show service logs (optional: logs [service])"
    echo "  test       - Test service connections"
    echo "  clean      - Stop services and delete all data"
    echo "  setup-env  - Create environment variables file"
    echo "  help       - Show this help message"
    echo ""
    echo "Examples:"
    echo "  $0 start           # Start all services"
    echo "  $0 logs localstack # Show LocalStack logs"
    echo "  $0 test           # Test connections"
}

# Main script logic
case "${1:-help}" in
    start)
        start_services
        ;;
    stop)
        stop_services
        ;;
    restart)
        restart_services
        ;;
    status)
        show_status
        ;;
    logs)
        show_logs $2
        ;;
    test)
        test_connections
        ;;
    clean)
        clean_data
        ;;
    setup-env)
        setup_env
        ;;
    help)
        show_help
        ;;
    *)
        print_error "Unknown command: $1"
        echo ""
        show_help
        exit 1
        ;;
esac
