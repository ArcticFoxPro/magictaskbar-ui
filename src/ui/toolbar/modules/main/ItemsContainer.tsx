export function ItemsDropableContainer({
  id,
  items,
}: {
  id: string;
  items: string[];
}) {
  return (
    <div className={`ft-bar-container ft-bar-${id}`}>
      {items.map((item) => (
        <div key={item} className="ft-bar-item" draggable>
          {item}
        </div>
      ))}
    </div>
  );
}
