#!/usr/bin/env python3
"""
YOLOv8 MPS推理脚本

基于PyTorch MPS后端在Apple Silicon GPU上运行YOLOv8目标检测
"""

import torch
import cv2
import numpy as np
import time
from pathlib import Path
import argparse
import sys

# 检查MPS可用性
if not torch.backends.mps.is_available():
    if not torch.backends.mps.is_built():
        print("MPS not available because the current PyTorch install was not built with MPS enabled.")
    else:
        print("MPS not available because the current MacOS version is not 12.3+ and/or you do not have an MPS-enabled device on this machine.")
    exit(1)

# 设置MPS设备
device = torch.device("mps")

def letterbox(im, new_shape=(640, 640), color=(114, 114, 114), auto=True, scaleup=True, stride=32):
    """调整图像大小并添加边框以适应目标尺寸"""
    shape = im.shape[:2]  # current shape [height, width]
    if isinstance(new_shape, int):
        new_shape = (new_shape, new_shape)

    # Scale ratio (new / old)
    r = min(new_shape[0] / shape[0], new_shape[1] / shape[1])
    if not scaleup:
        r = min(r, 1.0)

    # Compute padding
    new_unpad = int(round(shape[1] * r)), int(round(shape[0] * r))
    dw, dh = new_shape[1] - new_unpad[0], new_shape[0] - new_unpad[1]  # wh padding

    if auto:  # minimum rectangle
        dw, dh = np.mod(dw, stride), np.mod(dh, stride)  # wh padding

    dw /= 2  # divide padding into 2 sides
    dh /= 2

    if shape[::-1] != new_unpad:  # resize
        im = cv2.resize(im, new_unpad, interpolation=cv2.INTER_LINEAR)
    top, bottom = int(round(dh - 0.1)), int(round(dh + 0.1))
    left, right = int(round(dw - 0.1)), int(round(dw + 0.1))
    im = cv2.copyMakeBorder(im, top, bottom, left, right, cv2.BORDER_CONSTANT, value=color)  # add border
    return im, r, (dw, dh)

def xywh2xyxy(x):
    """Convert boxes from [x, y, w, h] to [x1, y1, x2, y2]"""
    y = x.clone() if isinstance(x, torch.Tensor) else np.copy(x)
    y[..., 0] = x[..., 0] - x[..., 2] / 2  # top left x
    y[..., 1] = x[..., 1] - x[..., 3] / 2  # top left y
    y[..., 2] = x[..., 0] + x[..., 2] / 2  # bottom right x
    y[..., 3] = x[..., 1] + x[..., 3] / 2  # bottom right y
    return y

def box_iou(box1, box2):
    """Calculate IoU between two sets of boxes"""
    def box_area(box):
        return (box[2] - box[0]) * (box[3] - box[1])

    area1 = box_area(box1.T)
    area2 = box_area(box2.T)

    # Intersection area
    inter = (torch.min(box1[:, None, 2:], box2[:, 2:]) - torch.max(box1[:, None, :2], box2[:, :2])).clamp(0).prod(2)
    return inter / (area1[:, None] + area2 - inter)  # IoU

def non_max_suppression(prediction, conf_thres=0.25, iou_thres=0.45, classes=None, agnostic=False, multi_label=False, labels=()):
    """Non-Maximum Suppression (NMS)"""
    nc = prediction.shape[2] - 5  # number of classes
    xc = prediction[..., 4] > conf_thres  # candidates

    # Settings
    min_wh, max_wh = 2, 4096  # (pixels) minimum and maximum box width and height
    max_det = 300  # maximum number of detections per image
    max_nms = 30000  # maximum number of boxes into torchvision.ops.nms()
    time_limit = 10.0  # seconds to quit after

    output = [torch.zeros((0, 6), device=prediction.device)] * prediction.shape[0]
    for xi, x in enumerate(prediction):  # image index, image inference
        x = x[xc[xi]]  # confidence

        # If no box remains
        if not x.shape[0]:
            continue

        # Compute conf
        x[:, 5:] *= x[:, 4:5]  # conf = obj_conf * cls_conf

        # Box (center x, center y, width, height) to (x1, y1, x2, y2)
        box = xywh2xyxy(x[:, :4])

        # Detections matrix nx6 (xyxy, conf, cls)
        if multi_label:
            i, j = (x[:, 5:] > conf_thres).nonzero(as_tuple=False).T
            x = torch.cat((box[i], x[i, j + 5, None], j[:, None].float()), 1)
        else:  # best class only
            conf, j = x[:, 5:].max(1, keepdim=True)
            x = torch.cat((box, conf, j.float()), 1)[conf.view(-1) > conf_thres]

        # Filter by class
        if classes is not None:
            x = x[(x[:, 5:6] == torch.tensor(classes, device=x.device)).any(1)]

        # Check shape
        n = x.shape[0]  # number of boxes
        if not n:  # no boxes
            continue
        elif n > max_nms:  # excess boxes
            x = x[x[:, 4].argsort(descending=True)[:max_nms]]  # sort by confidence

        # Batched NMS
        c = x[:, 5:6] * (0 if agnostic else max_wh)  # classes
        boxes, scores = x[:, :4] + c, x[:, 4]  # boxes (offset by class), scores
        i = torch.ops.torchvision.nms(boxes, scores, iou_thres)  # NMS
        if i.shape[0] > max_det:  # limit detections
            i = i[:max_det]
        output[xi] = x[i]
        
    return output

class YOLOv8MPSInference:
    def __init__(self, model_path, conf_threshold=0.5, iou_threshold=0.45):
        """Initialize YOLOv8 MPS inference"""
        self.model_path = model_path
        self.conf_threshold = conf_threshold
        self.iou_threshold = iou_threshold
        
        # Load model
        print(f"Loading model from {model_path}...")
        self.model = torch.hub.load('ultralytics/yolov8', 'custom', path=model_path, force_reload=True)
        self.model.to(device)
        self.model.eval()
        print(f"Model loaded successfully on {device}")
        
        # COCO classes
        self.classes = [
            'person', 'bicycle', 'car', 'motorcycle', 'airplane', 'bus', 'train', 'truck', 'boat',
            'traffic light', 'fire hydrant', 'stop sign', 'parking meter', 'bench', 'bird', 'cat',
            'dog', 'horse', 'sheep', 'cow', 'elephant', 'bear', 'zebra', 'giraffe', 'backpack',
            'umbrella', 'handbag', 'tie', 'suitcase', 'frisbee', 'skis', 'snowboard', 'sports ball',
            'kite', 'baseball bat', 'baseball glove', 'skateboard', 'surfboard', 'tennis racket',
            'bottle', 'wine glass', 'cup', 'fork', 'knife', 'spoon', 'bowl', 'banana', 'apple',
            'sandwich', 'orange', 'broccoli', 'carrot', 'hot dog', 'pizza', 'donut', 'cake', 'chair',
            'couch', 'potted plant', 'bed', 'dining table', 'toilet', 'tv', 'laptop', 'mouse', 'remote',
            'keyboard', 'cell phone', 'microwave', 'oven', 'toaster', 'sink', 'refrigerator', 'book',
            'clock', 'vase', 'scissors', 'teddy bear', 'hair drier', 'toothbrush'
        ]
    
    def preprocess(self, image):
        """Preprocess image for YOLOv8 inference"""
        # Convert BGR to RGB
        img_rgb = cv2.cvtColor(image, cv2.COLOR_BGR2RGB)
        
        # Letterbox resize
        img_resized, ratio, pad = letterbox(img_rgb, new_shape=(640, 640))
        
        # Convert to tensor
        img_tensor = torch.from_numpy(img_resized).permute(2, 0, 1).float() / 255.0
        img_tensor = img_tensor.unsqueeze(0)  # Add batch dimension
        
        return img_tensor.to(device), ratio, pad
    
    def postprocess(self, preds, ratio, pad):
        """Postprocess predictions"""
        # Apply NMS
        preds = non_max_suppression(preds, self.conf_threshold, self.iou_threshold)
        
        # Process detections
        detections = []
        if len(preds) > 0 and preds[0] is not None:
            det = preds[0]
            for *xyxy, conf, cls in reversed(det):
                # Convert to original image coordinates
                xyxy = torch.tensor(xyxy).view(-1)
                xyxy[0] = (xyxy[0] - pad[0]) / ratio
                xyxy[1] = (xyxy[1] - pad[1]) / ratio
                xyxy[2] = (xyxy[2] - pad[0]) / ratio
                xyxy[3] = (xyxy[3] - pad[1]) / ratio
                
                detections.append({
                    'class_id': int(cls),
                    'class_name': self.classes[int(cls)],
                    'confidence': float(conf),
                    'bbox': [float(xyxy[0]), float(xyxy[1]), float(xyxy[2]), float(xyxy[3])]
                })
        
        return detections
    
    def infer(self, image_path):
        """Perform inference on an image"""
        # Load image
        image = cv2.imread(str(image_path))
        if image is None:
            raise ValueError(f"Could not load image from {image_path}")
        
        # Preprocess
        img_tensor, ratio, pad = self.preprocess(image)
        
        # Inference
        start_time = time.time()
        with torch.no_grad():
            preds = self.model(img_tensor)
        inference_time = (time.time() - start_time) * 1000  # ms
        
        # Postprocess
        detections = self.postprocess(preds[0].cpu(), ratio, pad)
        
        return {
            'detections': detections,
            'inference_time_ms': inference_time,
            'image_shape': image.shape
        }
    
    def draw_detections(self, image, detections):
        """Draw detection results on image"""
        img_result = image.copy()
        
        for det in detections:
            # Get bounding box coordinates
            x1, y1, x2, y2 = map(int, det['bbox'])
            
            # Draw rectangle
            cv2.rectangle(img_result, (x1, y1), (x2, y2), (0, 255, 0), 2)
            
            # Draw label
            label = f"{det['class_name']} {det['confidence']:.2f}"
            cv2.putText(img_result, label, (x1, y1 - 10), cv2.FONT_HERSHEY_SIMPLEX, 0.5, (0, 255, 0), 2)
        
        return img_result

def main():
    parser = argparse.ArgumentParser(description='YOLOv8 MPS Inference')
    parser.add_argument('--model', type=str, required=True, help='Path to YOLOv8 model file (.pt)')
    parser.add_argument('--image', type=str, required=True, help='Path to input image')
    parser.add_argument('--output', type=str, default='result.jpg', help='Path to output image')
    parser.add_argument('--conf', type=float, default=0.5, help='Confidence threshold')
    parser.add_argument('--iou', type=float, default=0.45, help='IoU threshold for NMS')
    
    args = parser.parse_args()
    
    # Validate inputs
    if not Path(args.model).exists():
        print(f"Error: Model file {args.model} not found")
        sys.exit(1)
    
    if not Path(args.image).exists():
        print(f"Error: Image file {args.image} not found")
        sys.exit(1)
    
    try:
        # Initialize inference engine
        yolo = YOLOv8MPSInference(args.model, args.conf, args.iou)
        
        # Perform inference
        print(f"Processing image: {args.image}")
        result = yolo.infer(args.image)
        
        # Print results
        print(f"\nInference Results:")
        print(f"  Inference Time: {result['inference_time_ms']:.2f} ms")
        print(f"  Image Shape: {result['image_shape']}")
        print(f"  Detections Found: {len(result['detections'])}")
        
        for i, det in enumerate(result['detections']):
            print(f"    {i+1}. {det['class_name']} ({det['confidence']:.2f}) at [{det['bbox'][0]:.1f}, {det['bbox'][1]:.1f}, {det['bbox'][2]:.1f}, {det['bbox'][3]:.1f}]")
        
        # Draw and save results
        image = cv2.imread(args.image)
        result_img = yolo.draw_detections(image, result['detections'])
        cv2.imwrite(args.output, result_img)
        print(f"\nResult saved to: {args.output}")
        
    except Exception as e:
        print(f"Error during inference: {str(e)}")
        sys.exit(1)

if __name__ == "__main__":
    main()