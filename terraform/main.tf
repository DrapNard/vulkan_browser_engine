terraform {
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
  }
}

provider "aws" {
  region = var.aws_region
}

resource "aws_ecs_cluster" "vulkan_renderer" {
  name = "vulkan-renderer-cluster"
  
  setting {
    name  = "containerInsights"
    value = "enabled"
  }
}

resource "aws_ecs_task_definition" "vulkan_renderer" {
  family                   = "vulkan-renderer"
  network_mode             = "awsvpc"
  requires_compatibility   = ["FARGATE"]
  cpu                      = "1024"
  memory                   = "2048"
  execution_role_arn       = aws_iam_role.ecs_execution_role.arn

  container_definitions = jsonencode([
    {
      name  = "vulkan-renderer"
      image = "vulkan-renderer:latest"
      portMappings = [
        {
          containerPort = 8080
          protocol      = "tcp"
        }
      ]
      logConfiguration = {
        logDriver = "awslogs"
        options = {
          awslogs-group         = aws_cloudwatch_log_group.vulkan_renderer.name
          awslogs-region        = var.aws_region
          awslogs-stream-prefix = "ecs"
        }
      }
    }
  ])
}

resource "aws_ecs_service" "vulkan_renderer" {
  name            = "vulkan-renderer-service"
  cluster         = aws_ecs_cluster.vulkan_renderer.id
  task_definition = aws_ecs_task_definition.vulkan_renderer.arn
  desired_count   = 2
  launch_type     = "FARGATE"

  network_configuration {
    subnets         = var.private_subnets
    security_groups = [aws_security_group.vulkan_renderer.id]
  }

  load_balancer {
    target_group_arn = aws_lb_target_group.vulkan_renderer.arn
    container_name   = "vulkan-renderer"
    container_port   = 8080
  }
}