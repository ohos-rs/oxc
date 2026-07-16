declare let interface: PropertyDecorator
declare let other: PropertyDecorator
class Invalid {
  @interface
  @other
  value: number = 1
}

