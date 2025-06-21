output "load_balancer_dns" {
  description = "DNS name of the load balancer"
  value       = aws_lb.vulkan_renderer.dns_name
}

output "ecs_cluster_name" {
  description = "Name of the ECS cluster"
  value       = aws_ecs_cluster.vulkan_renderer.name
}

output "ecs_service_name" {
  description = "Name of the ECS service"
  value       = aws_ecs_service.vulkan_renderer.name
}