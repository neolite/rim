# Одноразовый x86-хост для замера rim и Alpine в настоящем Firecracker.
# c8i/m8i/r8i умеют вложенную виртуализацию (с 02.2026): /dev/kvm внутри обычной VM.
# Предохранители: инстанс сам выключается через max_hours и при выключении удаляется.
#
#   terraform init && terraform apply -var my_ip=$(curl -s https://checkip.amazonaws.com)/32
#   ... замер (scripts/fc-bench.sh) ...
#   terraform destroy

terraform {
  required_providers {
    aws = { source = "hashicorp/aws", version = ">= 6.67" }
  }
}

variable "profile" {
  type    = string
  default = null
}
variable "region" {
  type    = string
  default = "us-east-1"
}
variable "instance_type" {
  type    = string
  default = "c8i.large"
}
variable "my_ip" {
  type        = string
  description = "Ваш IP в виде a.b.c.d/32 — ssh только с него"
}
variable "ssh_public_key" {
  type    = string
  default = "~/.ssh/id_ed25519_rim.pub"
}
variable "max_hours" {
  type    = number
  default = 2
}

provider "aws" {
  region  = var.region
  profile = var.profile
  default_tags {
    tags = { Project = "rim-bench" }
  }
}

data "aws_ami" "ubuntu" {
  most_recent = true
  owners      = ["099720109477"] # Canonical
  filter {
    name   = "name"
    values = ["ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-amd64-server-*"]
  }
}

resource "aws_key_pair" "rim" {
  key_name_prefix = "rim-bench-"
  public_key      = file(pathexpand(var.ssh_public_key))
}

resource "aws_security_group" "ssh" {
  name_prefix = "rim-bench-"
  ingress {
    from_port   = 22
    to_port     = 22
    protocol    = "tcp"
    cidr_blocks = [var.my_ip]
  }
  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }
}

resource "aws_instance" "host" {
  ami                                  = data.aws_ami.ubuntu.id
  instance_type                        = var.instance_type
  key_name                             = aws_key_pair.rim.key_name
  vpc_security_group_ids               = [aws_security_group.ssh.id]
  instance_initiated_shutdown_behavior = "terminate"

  cpu_options {
    nested_virtualization = "enabled"
  }

  root_block_device {
    volume_size = 20
    volume_type = "gp3"
  }

  # Страховка от забытого инстанса: выключение (= удаление) через max_hours.
  user_data = "#!/bin/sh\nshutdown -h +${var.max_hours * 60}\n"

  tags = { Name = "rim-bench" }
}

output "ip" {
  value = aws_instance.host.public_ip
}
