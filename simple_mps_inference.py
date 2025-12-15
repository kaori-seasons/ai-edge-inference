#!/usr/bin/env python3
"""
简化版YOLOv8 MPS推理脚本

直接使用Ultralytics YOLOv8库进行MPS推理
"""

import torch
import cv2
import numpy as np
import time
from pathlib import Path
import argparse
import sys

def main():
    parser = argparse.ArgumentParser(description='Simple YOLOv8 MPS Inference')
    parser.add_argument('--image', type=str, required=True, help='Path to input image')
    parser.add_argument('--output', type=str, default='result.jpg', help='Path to output image')
    parser.add_argument('--conf', type=float, default=0.5, help='Confidence threshold')
    parser.add_argument('--iou', type=float, default=0.45, help='IoU threshold for NMS')
    
    args = parser.parse_args()
    
    # Validate input
    if not Path(args.image).exists():
        print(f"Error: Image file {args.image} not found")
        sys.exit(1)
    
    try:
        # Check MPS availability
        if not torch.backends.mps.is_available():
            print("MPS not available, using CPU")
            device = torch.device("cpu")
        else:
            print("Using MPS device")
            device = torch.device("mps")
        
        # Load YOLOv8 model (will automatically download if not present)
        print("Loading YOLOv8 model...")
        from ultralytics import YOLO
        model = YOLO('yolov8n.pt')  # nano model for fast inference
        model.to(device)
        print(f"Model loaded on {device}")
        
        # Perform inference
        print(f"Processing image: {args.image}")
        start_time = time.time()
        results = model(args.image, conf=args.conf, iou=args.iou, device=device)
        inference_time = (time.time() - start_time) * 1000  # ms
        
        # Extract results
        result = results[0]
        boxes = result.boxes
        
        # Print results
        print(f"\nInference Results:")
        print(f"  Inference Time: {inference_time:.2f} ms")
        print(f"  Detections Found: {len(boxes)}")
        
        # Save results
        result.save(filename=args.output)
        print(f"\nResult saved to: {args.output}")
        
        # Print detection details
        if len(boxes) > 0:
            class_names = result.names
            for i, box in enumerate(boxes):
                cls_id = int(box.cls[0])
                conf = float(box.conf[0])
                xyxy = box.xyxy[0].cpu().numpy()
                class_name = class_names[cls_id]
                print(f"    {i+1}. {class_name} ({conf:.2f}) at [{xyxy[0]:.1f}, {xyxy[1]:.1f}, {xyxy[2]:.1f}, {xyxy[3]:.1f}]")
        
    except ImportError:
        print("Error: ultralytics package not found. Please install it with:")
        print("  pip install ultralytics")
        sys.exit(1)
    except Exception as e:
        print(f"Error during inference: {str(e)}")
        sys.exit(1)

if __name__ == "__main__":
    main()